//! Saya binary options.
use clap::Parser;
use dojo_utils::keystore::prompt_password_if_needed;
use saya_core::starknet::account::StarknetAccountData;
use saya_core::SayaConfig;
use settlement::SettlementOptions;
use starknet::core::utils::cairo_short_string_to_felt;
use starknet::signers::SigningKey;
use starknet_account::StarknetAccountOptions;
use tracing::Subscriber;
use tracing_log::LogTracer;
use tracing_subscriber::{fmt, EnvFilter};
use url::Url;

use crate::args::proof::ProofOptions;

// mod data_availability;
mod proof;
mod settlement;
mod starknet_account;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
pub struct SayaArgs {
    /// Specify the Katana URL to fetch data from.
    #[arg(long)]
    #[arg(value_name = "RPC URL")]
    #[arg(help = "RPC URL to fetch data from.")]
    #[arg(default_value = "http://localhost:9545")]
    pub rpc_url: Url,

    /// Enable JSON logging.
    #[arg(long)]
    #[arg(help = "Output logs in JSON format.")]
    pub json_log: bool,

    /// Specify a block to start fetching data from.
    #[arg(short, long, default_value = "0")]
    pub start_block: u64,
    #[arg(short, long)]
    pub end_block: Option<u64>,

    #[arg(short, long, default_value = "1")]
    #[arg(help = "The number of blocks to be merged into a single proof.")]
    #[arg(conflicts_with = "end_block")]
    pub batch_size: usize,

    #[command(flatten)]
    #[command(next_help_heading = "Choose the saya execution mode")]
    pub settlement: SettlementOptions,

    #[command(flatten)]
    #[command(next_help_heading = "Choose the proof pipeline configuration")]
    pub proof: ProofOptions,

    #[command(flatten)]
    #[command(next_help_heading = "Starknet account configuration for settlement")]
    pub starknet_account: StarknetAccountOptions,
}

impl SayaArgs {
    pub fn init_logging(&self) -> Result<(), Box<dyn std::error::Error>> {
        const DEFAULT_LOG_FILTER: &str =
            "info,saya::core=trace,blockchain=off,provider=off,atlantic_client=off,log=off"; //log is off because its orgin-prove_block is too verbose

        LogTracer::init()?;

        let builder = fmt::Subscriber::builder().with_env_filter(
            EnvFilter::try_from_default_env().or(EnvFilter::try_new(DEFAULT_LOG_FILTER))?,
        );

        let subscriber: Box<dyn Subscriber + Send + Sync> = if self.json_log {
            Box::new(builder.json().finish())
        } else {
            Box::new(builder.finish())
        };

        Ok(tracing::subscriber::set_global_default(subscriber)?)
    }
}

impl TryFrom<SayaArgs> for SayaConfig {
    type Error = Box<dyn std::error::Error>;

    fn try_from(args: SayaArgs) -> Result<Self, Self::Error> {
        // Check if the private key is from keystore or provided directly to follow `sozo`
        // conventions.
        let private_key = if let Some(pk) = args.starknet_account.signer_key {
            pk
        } else if let Some(path) = args.starknet_account.signer_keystore_path {
            let password = prompt_password_if_needed(
                args.starknet_account.signer_keystore_password.as_deref(),
                false,
            )?;
            SigningKey::from_keystore(path, &password)?.secret_scalar()
        } else {
            return Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Could not find private key. Please specify the private key or path to the \
                 keystore file.",
            )));
        };

        let starknet_account = StarknetAccountData {
            starknet_url: args.starknet_account.starknet_url,
            chain_id: cairo_short_string_to_felt(&args.starknet_account.chain_id)?,
            signer_address: args.starknet_account.signer_address,
            signer_key: private_key,
        };

        let settlement_contract =
            if let Some(settlement_contract) = args.settlement.settlement_contract {
                settlement_contract
            } else {
                return Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Persistent mode has to have a `settlement_contract`.",
                )));
            };

        Ok(SayaConfig {
            rpc_url: args.rpc_url,
            prover_url: args.proof.prover_url,
            prover_key: args.proof.private_key,
            settlement_contract,
            starknet_account,
        })
    }
}
