use std::{io::Read, path::PathBuf, sync::Arc, time::Duration};

use anyhow::Result;
use clap::Parser;
use saya_core::{
    block_ingestor::ShardingIngestorBuilder, orchestrator::ShardingOrchestratorBuilder,
    prover::AtlanticSnosProverBuilder, service::Daemon, shard::AggregatorMockBuilder,
    storage::SqliteDb,
};
use starknet::{
    accounts::{Account, ExecutionEncoding, SingleOwnerAccount},
    providers::{
        jsonrpc::{HttpTransport, JsonRpcClient},
        Provider,
    },
    signers::{LocalWallet, SigningKey},
};
use starknet_types_core::felt::Felt;
use url::Url;

use crate::common::SAYA_DB_PATH;

const GRACEFUL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Parser)]
pub struct Sharding {
    /// Rollup network Starknet JSON-RPC URL (v0.7.1)
    #[clap(long, env)]
    rollup_rpc: Url,
    /// Path to the compiled Starknet OS program
    #[clap(long, env)]
    snos_program: PathBuf,
    /// Path to the database directory
    #[clap(long, env)]
    db_dir: Option<PathBuf>,
    /// Atlantic prover API key
    #[clap(long, env)]
    atlantic_key: String,
    /// Whether to mock the SNOS proof by extracting the output from the PIE and using it from a proof.
    #[clap(long)]
    mock_snos_from_pie: bool,
    /// Shard contract address
    #[clap(long)]
    shard_contract_address: Felt,
    #[clap(long)]
    game_contract_address: Felt,
    #[clap(long)]
    event_name: String,
    #[clap(long)]
    shard_id: u32,
    #[clap(long)]
    account_address: Felt,
    /// Settlement network account private key
    #[clap(long, env)]
    account_private_key: Felt,
}

impl Sharding {
    pub async fn run(self) -> Result<()> {
        let mut snos_file = std::fs::File::open(self.snos_program)?;
        let mut snos = Vec::with_capacity(snos_file.metadata()?.len() as usize);
        snos_file.read_to_end(&mut snos)?;

        let saya_path = self
            .db_dir
            .map(|db_dir| format!("{}/{}", db_dir.display(), SAYA_DB_PATH))
            .unwrap_or_else(|| SAYA_DB_PATH.to_string());

        let db = SqliteDb::new(&saya_path).await?;

        let block_ingestor_builder = ShardingIngestorBuilder::new(
            self.rollup_rpc.clone(),
            snos,
            db.clone(),
            1,
            self.game_contract_address,
            self.event_name,
        );

        let snos_prover_builder = AtlanticSnosProverBuilder::new(
            self.atlantic_key,
            self.mock_snos_from_pie,
            db.clone(),
            1,
        );

        let provider: Arc<JsonRpcClient<HttpTransport>> = Arc::new(JsonRpcClient::new(
            HttpTransport::new(self.rollup_rpc.clone()),
        ));
        let chain_id = provider.chain_id().await?;
        let signer =
            LocalWallet::from_signing_key(SigningKey::from_secret_scalar(self.account_private_key));

        let account = SingleOwnerAccount::new(
            provider,
            signer,
            self.account_address,
            chain_id,
            ExecutionEncoding::New,
        );

        let aggregator_builder =
            AggregatorMockBuilder::new(self.shard_id, account, self.shard_contract_address);

        let orchestrator = ShardingOrchestratorBuilder::new(
            block_ingestor_builder,
            snos_prover_builder,
            aggregator_builder,
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
