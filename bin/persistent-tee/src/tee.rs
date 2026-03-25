//! `persistent-tee tee start` — runs the TEE pipeline end-to-end.

use std::{path::PathBuf, time::Duration};

use anyhow::Result;
use clap::{Parser, Subcommand};
use saya_core::{
    block_ingestor::BlockInfo,
    block_ingestor::BatchingPollingBlockIngestorBuilder, orchestrator::TeeOrchestratorBuilder,
    prover::TeeProof,
    service::Daemon,
    storage::{BlockStatus, SqliteDb},
    tee::{IncompleteBatch, L1ToL2Message, L2ToL1Message, TeeAttestation, TeeBatchStatus, TeeStorage},
};

use crate::settlement::TeePiltoverSettlementBackendBuilder;
use crate::storage::TeeDb;
use log::{info, warn};
use starknet_types_core::felt::Felt;
use starknet::{
    core::types::{BlockId, BlockTag, ExecutionResult, FunctionCall, StarknetError},
    macros::selector,
    providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider, ProviderError},
};
use url::Url;

use crate::attestor::TeeAttestorBuilder;
use crate::common::SAYA_DB_PATH;
use crate::prover::TeeProverBuilder;

/// 10 seconds.
const GRACEFUL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

/// Default attestor poll interval.
const DEFAULT_ATTESTOR_POLL_INTERVAL_MS: u64 = 1_000;

#[derive(Debug, Parser)]
pub struct Tee {
    #[clap(subcommand)]
    command: Subcommands,
}

#[derive(Debug, Subcommand)]
enum Subcommands {
    /// Start Saya in TEE mode.
    Start(Start),
}

#[derive(Debug, Parser, Clone)]
struct Start {
    /// Rollup network Starknet JSON-RPC URL (v0.7.1)
    #[clap(long, env)]
    rollup_rpc: Url,
    /// Settlement network Starknet JSON-RPC URL (v0.7.1)
    #[clap(long, env)]
    settlement_rpc: Url,
    /// Settlement network piltover contract address
    #[clap(long, env)]
    settlement_piltover_address: Felt,
    /// Settlement network account contract address
    #[clap(long, env)]
    settlement_account_address: Felt,
    /// Settlement network account private key
    #[clap(long, env)]
    settlement_account_private_key: Felt,
    /// TEE registry contract address on the prover network
    #[clap(long, env)]
    tee_registry_address: Felt,
    /// Private key for the prover network account
    #[clap(long, env)]
    prover_private_key: String,
    /// Attestor poll interval in milliseconds
    #[clap(long, env, default_value_t = DEFAULT_ATTESTOR_POLL_INTERVAL_MS)]
    attestor_poll_interval_ms: u64,
    /// Number of blocks to accumulate per TEE attestation batch
    #[clap(long, env, default_value_t = 10)]
    batch_size: usize,
    /// Flush a partial batch after this many seconds without a new block
    #[clap(long, env, default_value_t = 120)]
    idle_timeout_secs: u64,
    /// Path to the database directory
    #[clap(long, env)]
    db_dir: Option<PathBuf>,
}

impl Tee {
    pub async fn run(self) -> Result<()> {
        match self.command {
            Subcommands::Start(start) => start.run().await,
        }
    }
}

impl Start {
    async fn get_onchain_settled_block(&self) -> Result<u64> {
        let provider = JsonRpcClient::new(HttpTransport::new(self.settlement_rpc.clone()));
        let raw = provider
            .call(
                FunctionCall {
                    contract_address: self.settlement_piltover_address,
                    entry_point_selector: selector!("get_state"),
                    calldata: vec![],
                },
                BlockId::Tag(BlockTag::Latest),
            )
            .await?;

        let block_number = raw
            .get(1)
            .copied()
            .ok_or_else(|| anyhow::anyhow!("get_state returned fewer than 2 felts"))?;

        Ok(u64::try_from(block_number)
            .map_err(|_| anyhow::anyhow!("on-chain block_number does not fit into u64"))?)
    }

    fn blocks_from_range(batch: &IncompleteBatch) -> Vec<BlockInfo> {
        (batch.first_block..=batch.last_block)
            .map(|number| BlockInfo {
                number,
                status: BlockStatus::Mined,
                state_update: None,
                tee_batch_id: Some(batch.batch_id),
            })
            .collect()
    }

    fn attestation_from_stored(batch: &IncompleteBatch) -> Result<TeeAttestation> {
        let stored = batch
            .attestation
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing stored attestation"))?;

        Ok(TeeAttestation {
            batch_id: batch.batch_id,
            blocks: Self::blocks_from_range(batch),
            quote: stored.quote.clone(),
            prev_state_root: stored.prev_state_root.clone(),
            state_root: stored.state_root.clone(),
            prev_block_hash: stored.prev_block_hash.clone(),
            block_hash: stored.block_hash.clone(),
            prev_block_number: Felt::from_hex(&stored.prev_block_number)?,
            block_number: Felt::from_hex(&stored.block_number)?,
            messages_commitment: Felt::from_hex(&stored.messages_commitment)?,
            l2_to_l1_messages: serde_json::from_str::<Vec<L2ToL1Message>>(
                &stored.l2_to_l1_messages,
            )?,
            l1_to_l2_messages: serde_json::from_str::<Vec<L1ToL2Message>>(
                &stored.l1_to_l2_messages,
            )?,
        })
    }

    fn proof_from_stored(batch: &IncompleteBatch) -> Result<TeeProof> {
        let attestation = Self::attestation_from_stored(batch)?;
        let proof_bytes = batch
            .proof
            .clone()
            .ok_or_else(|| anyhow::anyhow!("missing stored proof bytes"))?;

        Ok(TeeProof {
            batch_id: batch.batch_id,
            blocks: attestation.blocks,
            data: proof_bytes,
            prev_state_root: Felt::from_hex_unchecked(&attestation.prev_state_root),
            state_root: Felt::from_hex(&attestation.state_root)?,
            prev_block_hash: Felt::from_hex(&attestation.prev_block_hash)?,
            block_hash: Felt::from_hex(&attestation.block_hash)?,
            prev_block_number: attestation.prev_block_number,
            block_number: attestation.block_number,
            messages_commitment: attestation.messages_commitment,
            l2_to_l1_messages: attestation.l2_to_l1_messages,
            l1_to_l2_messages: attestation.l1_to_l2_messages,
        })
    }

    async fn is_settlement_tx_confirmed(&self, tx_hash: &str) -> Result<bool> {
        let provider = JsonRpcClient::new(HttpTransport::new(self.settlement_rpc.clone()));
        let tx_hash = Felt::from_hex(tx_hash)?;

        match provider.get_transaction_receipt(tx_hash).await {
            Ok(receipt) => Ok(match receipt.receipt.execution_result() {
                ExecutionResult::Succeeded => true,
                ExecutionResult::Reverted { .. } => false,
            }),
            Err(ProviderError::StarknetError(StarknetError::TransactionHashNotFound)) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    pub async fn run(self) -> Result<()> {
        let saya_path = self
            .db_dir
            .as_ref()
            .map(|db_dir| format!("{}/{}", db_dir.display(), SAYA_DB_PATH))
            .unwrap_or_else(|| SAYA_DB_PATH.to_string());

        let db = SqliteDb::new(&saya_path).await?;
        let tee_db = TeeDb::new(&saya_path).await?;

        let onchain_settled_block = self.get_onchain_settled_block().await?;
        let mut recovered_attestations = Vec::new();
        let mut recovered_proofs = Vec::new();

        for batch in tee_db.get_incomplete_batches().await? {
            if batch.last_block <= onchain_settled_block {
                tee_db
                    .set_batch_status(batch.batch_id, TeeBatchStatus::Settled)
                    .await?;
                let _ = tee_db.confirm_settlement_tx(batch.batch_id).await;
                continue;
            }

            match batch.status {
                TeeBatchStatus::Attested => match Self::attestation_from_stored(&batch) {
                    Ok(attestation) => recovered_attestations.push(attestation),
                    Err(e) => warn!(batch_id = batch.batch_id, error:% = e; "Failed to reconstruct attested batch during startup recovery"),
                },
                TeeBatchStatus::Proved => match Self::proof_from_stored(&batch) {
                    Ok(proof) => recovered_proofs.push(proof),
                    Err(e) => warn!(batch_id = batch.batch_id, error:% = e; "Failed to reconstruct proved batch during startup recovery"),
                },
                TeeBatchStatus::SettlementPending => {
                    let confirmed = match &batch.settlement_tx_hash {
                        Some(tx_hash) => self.is_settlement_tx_confirmed(tx_hash).await.unwrap_or(false),
                        None => false,
                    };

                    if confirmed {
                        tee_db.confirm_settlement_tx(batch.batch_id).await?;
                        tee_db
                            .set_batch_status(batch.batch_id, TeeBatchStatus::Settled)
                            .await?;
                    } else {
                        match Self::proof_from_stored(&batch) {
                            Ok(proof) => recovered_proofs.push(proof),
                            Err(e) => warn!(batch_id = batch.batch_id, error:% = e; "Failed to reconstruct settlement_pending batch during startup recovery"),
                        }
                    }
                }
                _ => {}
            }
        }

        info!(
            recovered_attestations = recovered_attestations.len(),
            recovered_proofs = recovered_proofs.len(),
            onchain_settled_block;
            "Startup recovery prepared recovered batches"
        );

        let block_ingestor_builder = BatchingPollingBlockIngestorBuilder::new(
            self.rollup_rpc.clone(),
            db.clone(),
            self.batch_size,
            Duration::from_secs(self.idle_timeout_secs),
            Some(tee_db.clone()),
        );

        let attestor_builder = TeeAttestorBuilder::new(
            self.rollup_rpc.clone(),
            Duration::from_millis(self.attestor_poll_interval_ms),
            tee_db.clone(),
        );

        let prover_builder = TeeProverBuilder::new(
            self.settlement_rpc.to_string(),
            self.tee_registry_address,
            self.prover_private_key,
            tee_db.clone(),
        );

        let settlement_builder = TeePiltoverSettlementBackendBuilder::new(
            self.settlement_rpc,
            self.settlement_piltover_address,
            self.settlement_account_address,
            self.settlement_account_private_key,
            tee_db,
        );

        let orchestrator = TeeOrchestratorBuilder::new(
            block_ingestor_builder,
            attestor_builder,
            prover_builder,
            settlement_builder,
        )
        .recovered_attestations(recovered_attestations)
        .recovered_proofs(recovered_proofs)
        .build()
        .await?;

        let orchestrator_shutdown = orchestrator.shutdown_handle();
        orchestrator.start();

        let mut sigterm_handle =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let ctrl_c_handle = tokio::signal::ctrl_c();

        tokio::select! {
            _ = sigterm_handle.recv() => {},
            _ = ctrl_c_handle => {},
            _ = orchestrator_shutdown.finished() => {},
        }

        // Graceful shutdown
        orchestrator_shutdown.shutdown();
        tokio::select! {
            _ = tokio::time::sleep(GRACEFUL_SHUTDOWN_TIMEOUT) => {
                Err(anyhow::anyhow!("timeout waiting for graceful shutdown"))
            },
            _ = orchestrator_shutdown.finished() => {
                Ok(())
            },
        }
    }
}
