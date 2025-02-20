use std::{io::Read, path::PathBuf, time::Duration};

use anyhow::Result;
use clap::{Parser, Subcommand};
use prover_sdk::access_key::ProverAccessKey;
use saya_core::{
    block_ingestor::{
        pie_generator::{local::LocalPieGenerator, remote::RemotePieGenerator, SnosPieGenerator},
        PollingBlockIngestorBuilder,
    },
    data_availability::CelestiaDataAvailabilityBackendBuilder,
    orchestrator::{Genesis, SovereignOrchestratorBuilder},
    prover::AtlanticSnosProverBuilder,
    service::Daemon,
    storage::InMemoryStorageBackend,
};
use url::Url;

/// 10 seconds.
const GRACEFUL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Parser)]
pub struct Sovereign {
    #[clap(subcommand)]
    command: Subcommands,
}

#[derive(Debug, Subcommand)]
enum Subcommands {
    /// Start Saya in sovereign mode.
    Start(Start),
}

#[derive(Debug, Parser)]
struct Start {
    /// Starknet JSON-RPC URL (v0.7.1)
    #[clap(long, env)]
    starknet_rpc: Url,
    /// Path to the compiled Starknet OS program
    #[clap(long, env)]
    snos_program: PathBuf,
    /// Whether to mock the SNOS proof by extracting the output from the PIE and using it from a proof.
    #[clap(long)]
    mock_snos_from_pie: bool,
    /// Atlantic prover API key
    #[clap(long, env)]
    atlantic_key: String,
    /// Celestia RPC endpoint URL
    #[clap(long, env)]
    celestia_rpc: Url,
    /// Celestia RPC node auth token
    #[clap(long, env)]
    celestia_token: String,
    #[clap(flatten)]
    genesis: GenesisOptions,
    #[clap(subcommand)]
    pie_mode: PieGenerationMode,
}

#[derive(Debug, Parser)]
struct GenesisOptions {
    #[clap(
        long = "genesis.first-block-number",
        env = "GENESIS_FIRST_BLOCK_NUMBER"
    )]
    first_block_number: Option<u64>,
}

#[derive(Debug, Subcommand)]
enum PieGenerationMode {
    Local,
    Remote {
        /// Remote prover URL
        #[clap(long, env)]
        url: Url,
        /// Remote prover API access key
        #[clap(long, env)]
        access_key: String,
    },
}
impl From<PieGenerationMode> for SnosPieGenerator {
    fn from(pie_mode: PieGenerationMode) -> Self {
        match pie_mode {
            PieGenerationMode::Local => SnosPieGenerator::Local(LocalPieGenerator),
            PieGenerationMode::Remote { url, access_key } => {
                SnosPieGenerator::Remote(Box::new(RemotePieGenerator {
                    url: url.to_string(),
                    access_key: ProverAccessKey::from_hex_string(&access_key)
                        .expect("Invalid access key"), // You might want to handle this error better
                }))
            }
        }
    }
}

impl Sovereign {
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

        // TODO: make impls of these providers configurable
        let pie_gen: SnosPieGenerator = self.pie_mode.into();
        let block_ingestor_builder =
            PollingBlockIngestorBuilder::new(self.starknet_rpc, snos, pie_gen);
        let prover_builder =
            AtlanticSnosProverBuilder::new(self.atlantic_key, self.mock_snos_from_pie);
        let da_builder =
            CelestiaDataAvailabilityBackendBuilder::new(self.celestia_rpc, self.celestia_token);
        let storage = InMemoryStorageBackend::new();

        let orchestrator = SovereignOrchestratorBuilder::new(
            block_ingestor_builder,
            prover_builder,
            da_builder,
            storage,
            self.genesis.into(),
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

impl From<GenesisOptions> for Option<Genesis> {
    fn from(value: GenesisOptions) -> Self {
        value.first_block_number.map(|num| Genesis {
            first_block_number: num,
        })
    }
}
