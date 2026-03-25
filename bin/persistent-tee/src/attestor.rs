//! TEE attestor — fetches attestation data from the Katana rollup node.
//!
//! The attestor receives an ordered batch of [`BlockInfo`] from the upstream ingestor and
//! fetches a single TEE attestation covering the entire batch from the Katana JSON-RPC.

use std::time::Duration;

use anyhow::Result;
use katana_tee_client::KatanaRpcClient;
use log::{debug, error, info, warn};
use saya_core::tee::{StoredAttestation, TeeBatchStatus, TeeStorage};
use tokio::sync::mpsc::{Receiver, Sender};
use url::Url;

use crate::common::{retry_with_backoff, RETRY_INITIAL_DELAY, STAGE_MAX_ATTEMPTS};
use crate::storage::TeeDb;

#[allow(unused_imports)]
use saya_core::prover::HasBlockNumber;
use saya_core::{
    block_ingestor::BlockInfo,
    prover::{PipelineStage, PipelineStageBuilder},
    service::{Daemon, FinishHandle, ShutdownHandle},
    tee::{L1ToL2Message, L2ToL1Message, TeeAttestation},
};

/// Fetches TEE attestation from the Katana rollup node for each incoming batch of
/// [`BlockInfo`].
#[derive(Debug)]
pub struct TeeAttestor {
    katana_rpc: Url,
    _poll_interval: Duration,
    db: TeeDb,
    input_channel: Receiver<Vec<BlockInfo>>,
    output_channel: Sender<TeeAttestation>,
    finish_handle: FinishHandle,
}

#[derive(Debug)]
pub struct TeeAttestorBuilder {
    katana_rpc: Url,
    poll_interval: Duration,
    db: TeeDb,
    input_channel: Option<Receiver<Vec<BlockInfo>>>,
    output_channel: Option<Sender<TeeAttestation>>,
}

impl TeeAttestorBuilder {
    pub fn new(katana_rpc: Url, poll_interval: Duration, db: TeeDb) -> Self {
        Self {
            katana_rpc,
            poll_interval,
            db,
            input_channel: None,
            output_channel: None,
        }
    }
}

impl PipelineStageBuilder for TeeAttestorBuilder {
    type Stage = TeeAttestor;

    fn build(self) -> Result<Self::Stage> {
        Ok(TeeAttestor {
            katana_rpc: self.katana_rpc,
            _poll_interval: self.poll_interval,
            db: self.db,
            input_channel: self
                .input_channel
                .ok_or_else(|| anyhow::anyhow!("`input_channel` not set"))?,
            output_channel: self
                .output_channel
                .ok_or_else(|| anyhow::anyhow!("`output_channel` not set"))?,
            finish_handle: FinishHandle::new(),
        })
    }

    fn input_channel(mut self, input_channel: Receiver<Vec<BlockInfo>>) -> Self {
        self.input_channel = Some(input_channel);
        self
    }

    fn output_channel(mut self, output_channel: Sender<TeeAttestation>) -> Self {
        self.output_channel = Some(output_channel);
        self
    }
}

impl PipelineStage for TeeAttestor {
    type Input = Vec<BlockInfo>;
    type Output = TeeAttestation;
}

impl Daemon for TeeAttestor {
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        tokio::spawn(self.run());
    }
}

impl TeeAttestor {
    async fn run(mut self) {
        loop {
            let blocks = tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                batch = self.input_channel.recv() => match batch {
                    Some(b) => b,
                    None => break,
                },
            };
            info!(
                first_block = blocks.first().map(|b| b.number),
                last_block = blocks.last().map(|b| b.number),
                count = blocks.len();
                "Fetching TEE attestation for block batch"
            );

            let batch_id = match blocks.first().and_then(|b| b.tee_batch_id) {
                Some(id) => id,
                None => {
                    error!("Missing tee_batch_id on ingested block batch");
                    continue;
                }
            };

            let mut scratch_attempt = 0u32;
            let attestation = 'scratch: loop {
                let attempt_blocks = blocks.clone();
                match retry_with_backoff(
                    "attestor_fetch",
                    STAGE_MAX_ATTEMPTS,
                    RETRY_INITIAL_DELAY,
                    || {
                        let blocks = attempt_blocks.clone();
                        async { self.fetch_attestation(blocks).await }
                    },
                )
                .await
                {
                    Ok(a) => break 'scratch a,
                    Err(e) => {
                        warn!(batch_id, error:% = e; "Attestor exhausted retries");
                        if let Err(db_err) = self
                            .db
                            .set_batch_status(batch_id, TeeBatchStatus::Failed)
                            .await
                        {
                            error!(batch_id, error:% = db_err; "Failed to mark batch failed");
                        }
                        if let Err(db_err) = self.db.increment_retry_count(batch_id).await {
                            error!(batch_id, error:% = db_err; "Failed to increment retry_count");
                        }
                        scratch_attempt += 1;
                        if scratch_attempt >= 2 {
                            error!(batch_id, error:% = e; "Attestor scratch retry also failed; exiting process");
                            std::process::exit(1);
                        }
                        warn!(batch_id; "Retrying batch once from scratch at attestor stage");
                        if let Err(db_err) = self
                            .db
                            .set_batch_status(batch_id, TeeBatchStatus::PendingAttestation)
                            .await
                        {
                            error!(batch_id, error:% = db_err; "Failed to reset batch status for scratch retry");
                        }
                    }
                }
            };

            let stored = StoredAttestation {
                quote: attestation.quote.clone(),
                prev_state_root: attestation.prev_state_root.clone(),
                state_root: attestation.state_root.clone(),
                prev_block_hash: attestation.prev_block_hash.clone(),
                block_hash: attestation.block_hash.clone(),
                prev_block_number: attestation.prev_block_number.to_hex_string(),
                block_number: attestation.block_number.to_hex_string(),
                messages_commitment: attestation.messages_commitment.to_hex_string(),
                l2_to_l1_messages: serde_json::to_string(&attestation.l2_to_l1_messages)
                    .unwrap_or_else(|_| "[]".to_string()),
                l1_to_l2_messages: serde_json::to_string(&attestation.l1_to_l2_messages)
                    .unwrap_or_else(|_| "[]".to_string()),
            };

            if let Err(e) = self.db.save_attestation(attestation.batch_id, &stored).await {
                log::error!("Failed to persist attestation for batch {}: {}", attestation.batch_id, e);
                continue;
            }

            if let Err(e) = self
                .db
                .set_batch_status(attestation.batch_id, TeeBatchStatus::Attested)
                .await
            {
                log::error!(
                    "Failed to set batch {} status to attested: {}",
                    attestation.batch_id,
                    e
                );
                continue;
            }

            info!("Fetched TEE attestation for block batch, sending to next stage");
            tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                _ = self.output_channel.send(attestation) => {},
            }
        }

        debug!("TeeAttestor graceful shutdown finished");
        self.finish_handle.finish();
    }

    async fn fetch_attestation(&self, blocks: Vec<BlockInfo>) -> Result<TeeAttestation> {
        let batch_id = blocks
            .first()
            .and_then(|b| b.tee_batch_id)
            .ok_or_else(|| anyhow::anyhow!("Missing tee_batch_id on ingested block batch"))?;

        let rpc_client = KatanaRpcClient::new(self.katana_rpc.clone());
        let block_number = blocks.last().expect("non-empty batch").number;
        let prev_block_number = blocks.first().expect("non-empty batch").number;
        let prev_block = if prev_block_number == 0 {
            None
        } else {
            Some(prev_block_number.saturating_sub(1))
        };
        let attestation = rpc_client
            .fetch_attestation(prev_block, block_number)
            .await?;
        let l2_to_l1_messages = attestation
            .l2_to_l1_messages
            .into_iter()
            .map(|m| L2ToL1Message {
                from_address: m.from_address,
                to_address: m.to_address,
                payload: m.payload,
            })
            .collect();

        let l1_to_l2_messages = attestation
            .l1_to_l2_messages
            .into_iter()
            .map(|m| L1ToL2Message {
                from_address: m.from_address,
                to_address: m.to_address,
                selector: m.selector,
                payload: m.payload,
                nonce: m.nonce,
            })
            .collect();

        Ok(TeeAttestation {
            batch_id,
            blocks,
            quote: attestation.quote,
            prev_state_root: attestation.prev_state_root,
            state_root: attestation.state_root,
            prev_block_hash: attestation.prev_block_hash,
            block_hash: attestation.block_hash,
            prev_block_number: attestation.prev_block_number,
            block_number: attestation.block_number,
            messages_commitment: attestation.messages_commitment,
            l2_to_l1_messages,
            l1_to_l2_messages,
        })
    }
}
