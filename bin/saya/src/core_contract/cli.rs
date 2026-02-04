use std::path::Path;

use crate::core_contract::constants::{
    ATLANTIC_FACT_REGISTRY_MAINNET, ATLANTIC_FACT_REGISTRY_SEPOLIA, DEFAULT_PILTOVER_CLASS_HASH,
    DEFAULT_PILTOVER_PATH, KATANA_STRK_FEE_TOKEN, MAINNET_RPC_URL, SEPOLIA_RPC_URL,
};
use crate::core_contract::utils::{
    compute_starknet_os_config_hash, declare_core_contract, deploy_core_contract,
    set_fact_registry, set_program_info,
};
use anyhow::Result;
use clap::ValueEnum;
use clap::{Args, Parser, Subcommand};
use log::info;
use starknet::core::types::Felt;
use starknet::core::utils::cairo_short_string_to_felt;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider};
use starknet::signers::{LocalWallet, SigningKey};
use url::Url;

/// Supported settlement chain options for rollup initialization.
#[derive(Debug, Clone, PartialEq, Eq, Parser, ValueEnum)]
enum SettlementChain {
    Mainnet,
    Sepolia,
    #[cfg(feature = "init-custom-settlement-chain")]
    Custom(Url),
}
#[derive(Debug, Parser)]
pub struct CoreContract {
    #[clap(subcommand)]
    cmd: CoreContractCmd,
    #[clap(long, env = "SETTLEMENT_ACCOUNT_PRIVATE_KEY")]
    private_key: String,
    #[clap(long, env = "SETTLEMENT_ACCOUNT_ADDRESS")]
    account_address: String,
    #[clap(long, env = "SETTLEMENT_CHAIN")]
    settlement_chain_id: SettlementChain,
}

#[derive(Debug, Subcommand)]
pub enum CoreContractCmd {
    Declare(DeclareArgs),
    Deploy(DeployArgs),
    SetupProgram(SetupProgramArgs),
}

#[derive(Debug, Args)]
pub struct SetupProgramArgs {
    #[clap(long, env = "FACT_REGISTRY_ADDRESS")]
    fact_registry_address: Option<Felt>,
    #[clap(long, env = "CORE_CONTRACT_ADDRESS")]
    core_contract_address: Felt,
    #[clap(long, env = "FEE_TOKEN_ADDRESS",default_value = KATANA_STRK_FEE_TOKEN.to_hex_string())]
    fee_token_address: Felt,
    #[clap(long, env = "CHAIN_ID")]
    chain_id: String,
}

#[derive(Debug, Args)]
pub struct DeployArgs {
    #[clap(long, env = "CLASS_HASH",default_value = DEFAULT_PILTOVER_CLASS_HASH.to_hex_string())]
    class_hash: Felt,
    #[clap(long, env = "SALT")]
    salt: Felt,
}

#[derive(Debug, Args)]
pub struct DeclareArgs {
    #[clap(long, env = "CORE_CONTRACT_PATH", default_value = DEFAULT_PILTOVER_PATH)]
    core_contract_path: String,
}

impl CoreContract {
    pub async fn run(self) -> Result<()> {
        let provider = self.get_provider();
        let signer: LocalWallet = LocalWallet::from_signing_key(SigningKey::from_secret_scalar(
            Felt::from_hex(&self.private_key).expect("Invalid private key"),
        ));

        let address = Felt::from_hex(&self.account_address).expect("Invalid address");

        let chain_id = provider.chain_id().await.expect("Failed to fetch chain id");
        let encoding = starknet::accounts::ExecutionEncoding::New;
        let account = starknet::accounts::SingleOwnerAccount::new(
            provider,
            signer.clone(),
            address,
            chain_id,
            encoding,
        );
        match self.cmd {
            CoreContractCmd::Declare(declare_args) => {
                let class_hash = declare_core_contract(
                    account.clone(),
                    Path::new(&declare_args.core_contract_path),
                )
                .await?;
                info!("Core contract class hash: {:?}", class_hash);
            }
            CoreContractCmd::Deploy(deploy_args) => {
                let contract_address =
                    deploy_core_contract(account.clone(), deploy_args.class_hash, deploy_args.salt)
                        .await?;

                info!("Core contract address: {:?}", contract_address);
            }
            CoreContractCmd::SetupProgram(ref setup_program_args) => {
                let chain_id = cairo_short_string_to_felt(&setup_program_args.chain_id)?;

                let snos_config_hash =
                    compute_starknet_os_config_hash(chain_id, setup_program_args.fee_token_address);
                info!("Starknet OS config hash: {:?}", snos_config_hash);
                let tx_res = set_program_info(
                    account.clone(),
                    setup_program_args.core_contract_address,
                    snos_config_hash,
                )
                .await?;
                let fact_registry =
                    self.get_fact_registry_address(setup_program_args.fact_registry_address);
                info!("Set program info transaction submitted: {:?}", tx_res);
                let tx_res = set_fact_registry(
                    account.clone(),
                    setup_program_args.core_contract_address,
                    fact_registry,
                )
                .await?;
                info!("Fact registry set transaction submitted: {:?}", tx_res);
            }
        }

        Ok(())
    }
    pub fn get_provider(&self) -> JsonRpcClient<HttpTransport> {
        match &self.settlement_chain_id {
            SettlementChain::Mainnet => {
                JsonRpcClient::new(HttpTransport::new(Url::parse(MAINNET_RPC_URL).unwrap()))
            }
            SettlementChain::Sepolia => {
                JsonRpcClient::new(HttpTransport::new(Url::parse(SEPOLIA_RPC_URL).unwrap()))
            }
            #[cfg(feature = "init-custom-settlement-chain")]
            SettlementChain::Custom(url) => JsonRpcClient::new(HttpTransport::new(url.clone())),
        }
    }
    pub fn get_fact_registry_address(&self, fact_registry_address: Option<Felt>) -> Felt {
        if let Some(addr) = fact_registry_address {
            return addr;
        }

        match &self.settlement_chain_id {
            SettlementChain::Mainnet => ATLANTIC_FACT_REGISTRY_MAINNET,
            SettlementChain::Sepolia => ATLANTIC_FACT_REGISTRY_SEPOLIA,
            #[cfg(feature = "init-custom-settlement-chain")]
            SettlementChain::Custom(_) => {
                panic!("Fact registry address must be provided for custom settlement chain");
            }
        }
    }
}
