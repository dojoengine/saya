use std::{io::Read, path::PathBuf, time::Duration};

use anyhow::Result;
use clap::{Parser, Subcommand};
use prover_sdk::access_key::ProverAccessKey;
use saya_core::{
    block_ingestor::{
        pie_generator::{local::LocalPieGenerator, remote::RemotePieGenerator, SnosPieGenerator},
        PollingBlockIngestorBuilder,
    },
    data_availability::NoopDataAvailabilityBackendBuilder,
    orchestrator::PersistentOrchestratorBuilder,
    prover::{
        trace::{AtlanticTraceGenerator, HttpProverTraceGen, TraceGenerator},
        AtlanticClient, AtlanticLayoutBridgeProverBuilder, AtlanticSnosProverBuilder,
        MockLayoutBridgeProverBuilder, RecursiveProverBuilder,
    },
    service::Daemon,
    settlement::PiltoverSettlementBackendBuilder,
};
use starknet_types_core::felt::Felt;
use url::Url;

use crate::any::AnyLayoutBridgeProverBuilder;

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

#[derive(Debug, Parser, Clone)]
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
    /// Whether to mock the SNOS proof by extracting the output from the PIE and using it from a proof.
    #[clap(long)]
    mock_snos_from_pie: bool,
    /// Path to the compiled Cairo verifier program
    #[clap(long, env)]
    layout_bridge_program: Option<PathBuf>,
    /// Atlantic prover API key
    #[clap(long, env)]
    atlantic_key: String,
    /// Settlement network integrity contract address
    #[clap(long, env)]
    settlement_integrity_address: Option<Felt>,
    /// Generate mock layout bridge proof and skip on-chain fact registration if provided
    #[clap(long, env)]
    mock_layout_bridge_program_hash: Option<Felt>,
    /// Settlement network piltover contract address
    #[clap(long, env)]
    settlement_piltover_address: Felt,
    /// Settlement network account contract address
    #[clap(long, env)]
    settlement_account_address: Felt,
    /// Settlement network account private key
    #[clap(long, env)]
    settlement_account_private_key: Felt,

    #[clap(subcommand)]
    pie_mode: PieGenerationMode,
}

#[derive(Debug, Subcommand, Clone)]
enum PieGenerationMode {
    /// Use local PIE generation
    Local,
    /// Use remote PIE generation (requires URL and access key)
    Remote {
        /// Remote prover URL
        #[clap(long, env)]
        url: Url,
        /// Remote prover API access key
        #[clap(long, env)]
        access_key: String,
    },
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
        let mut snos_file = std::fs::File::open(self.snos_program.clone())?;
        let mut snos = Vec::with_capacity(snos_file.metadata()?.len() as usize);
        snos_file.read_to_end(&mut snos)?;
        let trace_gen: TraceGenerator = self.clone().into();
        let layout_bridge_prover_builder =
            match (self.mock_layout_bridge_program_hash, self.layout_bridge_program) {
                // We don't need the `layout_bridge` program in this case but it's okay if it's given.
                (Some(mock_layout_bridge_program_hash), _) => {
                    AnyLayoutBridgeProverBuilder::Mock(MockLayoutBridgeProverBuilder::new(
                        mock_layout_bridge_program_hash,
                    ))
                }
                (None, Some(layout_bridge_program)) => {
                    let mut layout_bridge_file = std::fs::File::open(layout_bridge_program)?;
                    let mut layout_bridge =
                        Vec::with_capacity(layout_bridge_file.metadata()?.len() as usize);
                    layout_bridge_file.read_to_end(&mut layout_bridge)?;
                    AnyLayoutBridgeProverBuilder::Atlantic(AtlanticLayoutBridgeProverBuilder::new(
                        self.atlantic_key.clone(),
                        layout_bridge,
                        trace_gen,
                    ))
                }
                (None, None) => anyhow::bail!(
                    "invalid config: `--layout-bridge-program` must be provided unless `--mock-layout-bridge-program-hash` is used"
                ),
            };

        // TODO: make impls of these providers configurable
        let pie_gen: SnosPieGenerator = self.pie_mode.into();
        let block_ingestor_builder =
            PollingBlockIngestorBuilder::new(self.rollup_rpc, snos, pie_gen);
        let prover_builder = RecursiveProverBuilder::new(
            AtlanticSnosProverBuilder::new(self.atlantic_key, self.mock_snos_from_pie),
            layout_bridge_prover_builder,
        );
        let da_builder = NoopDataAvailabilityBackendBuilder::new();
        let settlement_builder = PiltoverSettlementBackendBuilder::new(
            self.settlement_rpc,
            self.settlement_piltover_address,
            self.settlement_account_address,
            self.settlement_account_private_key,
        );

        let settlement_builder = match (
            self.mock_layout_bridge_program_hash,
            self.settlement_integrity_address,
        ) {
            // We don't need `integrity` address but it's okay if it's given.
            (Some(_), _) => settlement_builder.skip_fact_registration(true),
            (None, Some(integrity_address)) => {
                settlement_builder.integrity_address(integrity_address)
            }
            (None, None) => anyhow::bail!(
                "invalid config: `integrity` address must be \
                provided unless `--mock-layout-bridge-program-hash` is used"
            ),
        };

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

impl From<Start> for TraceGenerator {
    fn from(value: Start) -> Self {
        match value.pie_mode {
            PieGenerationMode::Local => TraceGenerator::Atlantic(AtlanticTraceGenerator {
                atlantic_client: AtlanticClient::new(value.atlantic_key),
            }),
            PieGenerationMode::Remote { url, access_key } => {
                TraceGenerator::HttpProver(Box::new(HttpProverTraceGen {
                    url: url.to_string(),
                    access_key: ProverAccessKey::from_hex_string(&access_key)
                        .expect("Invalid access key"), // You might want to handle this error better
                }))
            }
        }
    }
}
