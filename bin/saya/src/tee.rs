use std::{path::PathBuf, time::Duration};

use anyhow::Result;
use clap::{Parser, Subcommand};
use saya_core::{
    block_ingestor::PollingBlockIngestorBuilder,
    orchestrator::TeeOrchestratorBuilder,
    prover::tee::TeeProverBuilder,
    service::Daemon,
    settlement::{PiltoverSettlementBackendBuilder, TeeFactRegistrar},
    storage::SqliteDb,
    tee::{OffchainTeeVerifierBuilder, TeeAttestorBuilder},
};
use starknet_types_core::felt::Felt;
use url::Url;

use crate::common::SAYA_DB_PATH;

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
    /// URL of the offchain TEE verifier service
    #[clap(long, env)]
    tee_verifier_url: Url,
    /// URL of the TEE proving service
    #[clap(long, env)]
    tee_prover_url: Url,
    /// Attestor poll interval in milliseconds
    #[clap(long, env, default_value_t = DEFAULT_ATTESTOR_POLL_INTERVAL_MS)]
    attestor_poll_interval_ms: u64,
    /// Path to the database directory
    #[clap(long, env)]
    db_dir: Option<PathBuf>,
    /// Number of block ingestor workers
    #[clap(long, env, default_value_t = 4)]
    ingestor_workers: usize,
}

impl Tee {
    pub async fn run(self) -> Result<()> {
        match self.command {
            Subcommands::Start(start) => start.run().await,
        }
    }
}

impl Start {
    pub async fn run(self) -> Result<()> {
        let saya_path = self
            .db_dir
            .map(|db_dir| format!("{}/{}", db_dir.display(), SAYA_DB_PATH))
            .unwrap_or_else(|| SAYA_DB_PATH.to_string());

        let db = SqliteDb::new(&saya_path).await?;

        let block_ingestor_builder = PollingBlockIngestorBuilder::new(
            self.rollup_rpc.clone(),
            db.clone(),
            self.ingestor_workers,
        );

        let attestor_builder = TeeAttestorBuilder::new(
            self.rollup_rpc,
            Duration::from_millis(self.attestor_poll_interval_ms),
        );

        let verifier_builder = OffchainTeeVerifierBuilder::new(self.tee_verifier_url);

        let prover_builder = TeeProverBuilder::new(self.tee_prover_url);

        let settlement_builder = PiltoverSettlementBackendBuilder::new(
            self.settlement_rpc,
            self.settlement_piltover_address,
            self.settlement_account_address,
            self.settlement_account_private_key,
            TeeFactRegistrar::new(self.settlement_piltover_address),
            db.clone(),
        );

        let orchestrator = TeeOrchestratorBuilder::new(
            block_ingestor_builder,
            attestor_builder,
            verifier_builder,
            prover_builder,
            settlement_builder,
        )
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
