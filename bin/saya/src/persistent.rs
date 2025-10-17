use std::{io::Read, path::PathBuf, time::Duration};

use anyhow::Result;
use clap::{Parser, Subcommand};
use saya_core::{
    block_ingestor::PollingBlockIngestorBuilder,
    data_availability::NoopDataAvailabilityBackendBuilder,
    orchestrator::PersistentOrchestratorBuilder,
    prover::{
        AtlanticLayoutBridgeProverBuilder, AtlanticSnosProverBuilder,
        MockLayoutBridgeProverBuilder, RecursiveProverBuilder,
    },
    service::Daemon,
    settlement::PiltoverSettlementBackendBuilder,
    storage::SqliteDb,
    ChainId, OsHintsConfiguration,
};
use starknet_types_core::felt::Felt;
use url::Url;

use crate::{
    any::AnyLayoutBridgeProverBuilder,
    common::{calculate_workers_per_stage, NUMBER_OF_STAGES, SAYA_DB_PATH},
};

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
    /// Path to the database directory
    #[clap(long, env)]
    db_dir: Option<PathBuf>,
    /// Number of blocks processed in parallel evenly distributed between the stages
    #[clap(long, env, default_value_t = 60)]
    blocks_processed_in_parallel: usize,
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
        let saya_path = self
            .db_dir
            .map(|db_dir| format!("{}/{}", db_dir.display(), SAYA_DB_PATH))
            .unwrap_or_else(|| SAYA_DB_PATH.to_string());

        let workers_distribution: [usize; NUMBER_OF_STAGES] =
            calculate_workers_per_stage(self.blocks_processed_in_parallel);
        let [snos_worker_count, layout_bridge_workers_count, ingestor_worker_count] =
            workers_distribution;

        log::info!(
            snos_worker_count,layout_bridge_workers_count,ingestor_worker_count;
            "workers distribution"
        );

        let db = SqliteDb::new(&saya_path).await?;
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
                        db.clone(),
                        layout_bridge_workers_count,
                    ))
                }
                (None, None) => anyhow::bail!(
                    "invalid config: `--layout-bridge-program` must be provided unless `--mock-layout-bridge-program-hash` is used"
                ),
            };

        // TODO: make impls of these providers configurable

        let block_ingestor_builder = PollingBlockIngestorBuilder::new(
            self.rollup_rpc,
            db.clone(),
            ingestor_worker_count,
            OsHintsConfiguration {
                debug_mode: false,
                full_output: false,
                use_kzg_da: false,
            },
            ChainId::Other("KATANA3".to_string()),
        );
        let prover_builder = RecursiveProverBuilder::new(
            AtlanticSnosProverBuilder::new(
                self.atlantic_key,
                self.mock_snos_from_pie,
                db.clone(),
                snos_worker_count,
            ),
            layout_bridge_prover_builder,
        );
        let da_builder = NoopDataAvailabilityBackendBuilder::new();
        let settlement_builder = PiltoverSettlementBackendBuilder::new(
            self.settlement_rpc,
            self.settlement_piltover_address,
            self.settlement_account_address,
            self.settlement_account_private_key,
            db.clone(),
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
