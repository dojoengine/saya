//! TEE prover — submits a TEE attestation to the SP1 proving service and retrieves the resulting
//! proof.

use crate::prover_impl::TeeAttestation as TeeAttestationProver;
use anyhow::Result;
use katana_tee_client::ProverConfig;
use katana_tee_client::TeeQuoteResponse;
use log::{debug, error, info, warn};
use saya_core::{
    prover::{HasBlockNumber, PipelineStage, PipelineStageBuilder, TeeProof},
    service::{Daemon, FinishHandle, ShutdownHandle},
    tee::{TeeAttestation, TeeBatchStatus, TeeStorage},
};
use starknet_types_core::felt::Felt;
use tokio::sync::mpsc::{Receiver, Sender};

use crate::common::{retry_with_backoff, RETRY_INITIAL_DELAY, STAGE_MAX_ATTEMPTS};
use crate::storage::TeeDb;

/// Submits a [`TeeAttestation`] to the TEE proving service and emits the resulting [`TeeProof`].
#[derive(Debug)]
pub struct TeeProver {
    provider_url: String,
    registry_address: Felt,
    private_key: String,
    db: TeeDb,
    input_channel: Receiver<TeeAttestation>,
    output_channel: Sender<TeeProof>,
    finish_handle: FinishHandle,
}

#[derive(Debug)]
pub struct TeeProverBuilder {
    provider_url: String,
    registry_address: Felt,
    private_key: String,
    db: TeeDb,
    input_channel: Option<Receiver<TeeAttestation>>,
    output_channel: Option<Sender<TeeProof>>,
}

impl TeeProverBuilder {
    pub fn new(
        provider_url: String,
        registry_address: Felt,
        private_key: String,
        db: TeeDb,
    ) -> Self {
        Self {
            provider_url,
            registry_address,
            private_key,
            db,
            input_channel: None,
            output_channel: None,
        }
    }
}

impl PipelineStageBuilder for TeeProverBuilder {
    type Stage = TeeProver;

    fn build(self) -> Result<Self::Stage> {
        Ok(TeeProver {
            provider_url: self.provider_url,
            registry_address: self.registry_address,
            private_key: self.private_key,
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

    fn input_channel(mut self, input_channel: Receiver<TeeAttestation>) -> Self {
        self.input_channel = Some(input_channel);
        self
    }

    fn output_channel(mut self, output_channel: Sender<TeeProof>) -> Self {
        self.output_channel = Some(output_channel);
        self
    }
}

impl PipelineStage for TeeProver {
    type Input = TeeAttestation;
    type Output = TeeProof;
}

impl Daemon for TeeProver {
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        tokio::spawn(self.run());
    }
}

impl TeeProver {
    async fn run(mut self) {
        loop {
            let attestation = tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                t = self.input_channel.recv() => match t {
                    Some(t) => t,
                    None => break,
                },
            };

            debug!(block_number = attestation.block_number(); "Submitting TEE attestation to prover");

            let batch_id = attestation.batch_id;

            let mut scratch_attempt = 0u32;
            let proof = 'scratch: loop {
                let attempt_attestation = attestation.clone();
                match retry_with_backoff(
                    "prover_generate",
                    STAGE_MAX_ATTEMPTS,
                    RETRY_INITIAL_DELAY,
                    || {
                        let attestation = attempt_attestation.clone();
                        async { self.prove(attestation).await }
                    },
                )
                .await
                {
                    Ok(p) => break 'scratch p,
                    Err(e) => {
                        warn!(batch_id, error:% = e; "Prover exhausted retries");
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
                            error!(batch_id, error:% = e; "Prover scratch retry also failed; exiting process");
                            std::process::exit(1);
                        }
                        // Attestation is still in DB; reset status so the batch
                        // is recoverable and re-prove with the same attestation.
                        warn!(batch_id; "Retrying batch once from scratch at prover stage");
                        if let Err(db_err) = self
                            .db
                            .set_batch_status(batch_id, TeeBatchStatus::Attested)
                            .await
                        {
                            error!(batch_id, error:% = db_err; "Failed to reset batch status for scratch retry");
                        }
                    }
                }
            };

            if let Err(e) = self.db.save_proof(proof.batch_id, &proof.data).await {
                log::error!("Failed to persist proof for batch {}: {}", proof.batch_id, e);
                continue;
            }

            if let Err(e) = self
                .db
                .set_batch_status(proof.batch_id, TeeBatchStatus::Proved)
                .await
            {
                log::error!("Failed to set batch {} status to proved: {}", proof.batch_id, e);
                continue;
            }

            tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                _ = self.output_channel.send(proof) => {},
            }
        }

        debug!("TeeProver graceful shutdown finished");
        self.finish_handle.finish();
    }

    async fn prove(&self, attestation: TeeAttestation) -> Result<TeeProof> {
        let block_number = attestation.block_number();

        info!(block_number; "TEE proving started for block batch");
        let response = TeeQuoteResponse {
            quote: attestation.quote.clone(),
            prev_state_root: attestation.prev_state_root.clone(),
            state_root: attestation.state_root.clone(),
            prev_block_hash: attestation.prev_block_hash.clone(),
            block_hash: attestation.block_hash.clone(),
            prev_block_number: attestation.prev_block_number,
            block_number: attestation.block_number,
            messages_commitment: attestation.messages_commitment,
            l2_to_l1_messages: vec![],
            l1_to_l2_messages: vec![],
        };

        let tee = TeeAttestationProver::from_response(&response)?;
        let config = ProverConfig {
            rpc_url: None,
            private_key: Some(self.private_key.clone()),
            skip_time_validity_check: false,
        };
        let proof = tee
            .generate_proof(&self.provider_url, self.registry_address, config)
            .await?;
        let proof_raw = proof.encode_json()?;
        info!(block_number; "TEE proving completed, proof size: {} bytes", proof_raw.len());

        let prev_state_root = Felt::from_hex_unchecked(&attestation.prev_state_root);
        let state_root = Felt::from_hex(&attestation.state_root)?;
        let prev_block_hash = Felt::from_hex(&attestation.prev_block_hash)?;
        let block_hash = Felt::from_hex(&attestation.block_hash)?;

        Ok(TeeProof {
            batch_id: attestation.batch_id,
            blocks: attestation.blocks,
            data: proof_raw,
            prev_state_root,
            state_root,
            prev_block_hash,
            block_hash,
            prev_block_number: attestation.prev_block_number,
            block_number: attestation.block_number,
            messages_commitment: attestation.messages_commitment,
            l2_to_l1_messages: attestation.l2_to_l1_messages,
            l1_to_l2_messages: attestation.l1_to_l2_messages,
        })
    }
}
