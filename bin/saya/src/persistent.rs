use std::{io::Read, path::PathBuf, time::Duration};

use anyhow::Result;
use clap::{Parser, Subcommand};
use saya_core::{
    block_ingestor::PollingBlockIngestorBuilder,
    data_availability::CelestiaDataAvailabilityBackendBuilder,
    orchestrator::PersistentOrchestratorBuilder,
    prover::{
        AtlanticLayoutBridgeProverBuilder, AtlanticSnosProverBuilder, RecursiveProverBuilder,
    },
    service::Daemon,
    settlement::PiltoverSettlementBackendBuilder,
};
use starknet_types_core::felt::Felt;
use url::Url;

/// 10 seconds.
const GRACEFUL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Parser)]
pub struct Persistent {
    #[clap(subcommand)]
    command: Subcommands,
}

#[derive(Debug, Subcommand)]
enum Subcommands {
    /// Start Saya in persistent mode.
    Start(Start),
}

#[derive(Debug, Parser)]
struct Start {
    /// Rollup network Starknet JSON-RPC URL (v0.7.1)
    #[clap(long, env)]
    rollup_rpc: Url,
    /// Settlement network Starknet JSON-RPC URL (v0.7.1)
    #[clap(long, env)]
    settlement_rpc: Url,
    /// Path to the compiled Starknet OS program
    #[clap(long, env)]
    snos_program: PathBuf,
    /// Path to the compiled Cairo verifier program
    #[clap(long, env)]
    layout_bridge_program: PathBuf,
    /// Atlantic prover API key
    #[clap(long, env)]
    atlantic_key: String,
    /// Celestia RPC endpoint URL
    #[clap(long, env)]
    celestia_rpc: Url,
    /// Celestia RPC node auth token
    #[clap(long, env)]
    celestia_token: String,
    /// Settlement network settlement contract address
    #[clap(long, env)]
    settlement_contract_address: Felt,
    /// Settlement network account contract address
    #[clap(long, env)]
    settlement_account_address: Felt,
    /// Settlement network account private key
    #[clap(long, env)]
    settlement_account_private_key: Felt,
}

impl Persistent {
    pub async fn run(self) -> Result<()> {
        match self.command {
            Subcommands::Start(start) => start.run().await,
        }
    }
}

impl Start {
    pub async fn run(self) -> Result<()> {
        let mut snos_file = std::fs::File::open(self.snos_program)?;
        let mut snos = Vec::with_capacity(snos_file.metadata()?.len() as usize);
        snos_file.read_to_end(&mut snos)?;

        let mut layout_bridge_file = std::fs::File::open(self.layout_bridge_program)?;
        let mut layout_bridge = Vec::with_capacity(layout_bridge_file.metadata()?.len() as usize);
        layout_bridge_file.read_to_end(&mut layout_bridge)?;

        // TODO: make impls of these providers configurable
        let block_ingestor_builder = PollingBlockIngestorBuilder::new(self.rollup_rpc, snos);
        let prover_builder = RecursiveProverBuilder::new(
            AtlanticSnosProverBuilder::new(self.atlantic_key.clone()),
            AtlanticLayoutBridgeProverBuilder::new(self.atlantic_key, layout_bridge),
        );
        let da_builder =
            CelestiaDataAvailabilityBackendBuilder::new(self.celestia_rpc, self.celestia_token);
        let settlement_builder = PiltoverSettlementBackendBuilder::new(
            self.settlement_rpc,
            self.settlement_contract_address,
            self.settlement_account_address,
            self.settlement_account_private_key,
        );

        let orchestrator = PersistentOrchestratorBuilder::new(
            block_ingestor_builder,
            prover_builder,
            da_builder,
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
