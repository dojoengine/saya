use std::path::Path;
use std::str::FromStr;

use crate::core_contract::constants::{
    ATLANTIC_FACT_REGISTRY_MAINNET, ATLANTIC_FACT_REGISTRY_SEPOLIA, DEFAULT_PILTOVER_CLASS_HASH,
    FACT_REGISTRY_MOCK_BYTES, KATANA_STRK_FEE_TOKEN, MAINNET_RPC_URL, PILTOVER_CONTRACT_BYTES,
    SEPOLIA_RPC_URL,
};
use crate::core_contract::utils::{
    compute_starknet_os_config_hash, declare_contract, declare_contract_from_bytes,
    deploy_contract, deploy_core_contract, set_fact_registry, set_program_info,
};
use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use log::info;
use starknet::core::types::Felt;
use starknet::core::utils::cairo_short_string_to_felt;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider};
use starknet::signers::{LocalWallet, SigningKey};
use url::Url;

/// Supported settlement chain options for rollup initialization.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SettlementChain {
    Mainnet,
    Sepolia,
    Custom(String),
}

impl FromStr for SettlementChain {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "mainnet" => Ok(SettlementChain::Mainnet),
            "sepolia" => Ok(SettlementChain::Sepolia),
            _ => Ok(SettlementChain::Custom(s.to_string())),
        }
    }
}
#[derive(Debug, Parser)]
pub struct CoreContract {
    #[clap(subcommand)]
    cmd: CoreContractCmd,
    #[clap(long, env = "SETTLEMENT_ACCOUNT_PRIVATE_KEY")]
    private_key: String,
    #[clap(long, env = "SETTLEMENT_ACCOUNT_ADDRESS")]
    account_address: String,
    #[clap(long, env = "SETTLEMENT_RPC_URL")]
    settlement_rpc_url: Option<Url>,
    #[clap(long, env = "SETTLEMENT_CHAIN_ID")]
    settlement_chain_id: SettlementChain,
}

#[derive(Debug, Subcommand)]
pub enum CoreContractCmd {
    Declare(DeclareArgs),
    Deploy(DeployArgs),
    DeclareAndDeployFactRegistryMock(DeployFactRegistryArgs),
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
pub struct DeployFactRegistryArgs {
    /// Path to the fact registry mock contract (optional, uses embedded contract by default)
    #[clap(long, env = "FACT_REGISTRY_PATH")]
    fact_registry_path: Option<String>,
    #[clap(long, env = "FACT_REGISTRY_SALT")]
    salt: Felt,
}

#[derive(Debug, Args)]
pub struct DeclareArgs {
    /// Path to the core contract (optional, uses embedded contract by default)
    #[clap(long, env = "CORE_CONTRACT_PATH")]
    core_contract_path: Option<String>,
}

impl CoreContract {
    /// Validates that custom chains have required parameters
    fn validate(&self) -> Result<()> {
        if let SettlementChain::Custom(chain_name) = &self.settlement_chain_id {
            if self.settlement_rpc_url.is_none() {
                anyhow::bail!(
                    "Settlement RPC URL is required for custom chain '{}'. \
                     Provide it via --settlement-rpc-url or SETTLEMENT_RPC_URL env var",
                    chain_name
                );
            }

            // For SetupProgram command, also check fact registry
            if let CoreContractCmd::SetupProgram(ref args) = self.cmd {
                if args.fact_registry_address.is_none() {
                    anyhow::bail!(
                        "Fact registry address is required for custom chain '{}'. \
                         Provide it via --fact-registry-address or FACT_REGISTRY_ADDRESS env var",
                        chain_name
                    );
                }
            }
        }

        Ok(())
    }

    pub async fn run(self) -> Result<()> {
        // Validate dependencies before proceeding
        self.validate()?;

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
                let class_hash = if let Some(path) = declare_args.core_contract_path {
                    declare_contract(account.clone(), "Core contract", Path::new(&path)).await?
                } else {
                    declare_contract_from_bytes(
                        account.clone(),
                        "Core contract",
                        PILTOVER_CONTRACT_BYTES,
                    )
                    .await?
                };

                info!("Core contract class hash: {:?}", class_hash);
            }
            CoreContractCmd::Deploy(deploy_args) => {
                let contract_address = deploy_core_contract(
                    account.clone(),
                    "Core contract",
                    deploy_args.class_hash,
                    deploy_args.salt,
                )
                .await?;

                info!("Core contract address: {:?}", contract_address);
            }
            CoreContractCmd::DeclareAndDeployFactRegistryMock(deploy_fact_registry_args) => {
                let class_hash = if let Some(path) = deploy_fact_registry_args.fact_registry_path {
                    declare_contract(account.clone(), "Fact registry mock", Path::new(&path))
                        .await?
                } else {
                    declare_contract_from_bytes(
                        account.clone(),
                        "Fact registry mock",
                        FACT_REGISTRY_MOCK_BYTES,
                    )
                    .await?
                };

                let fact_registry_address = deploy_contract(
                    account.clone(),
                    "Fact registry mock",
                    class_hash,
                    deploy_fact_registry_args.salt,
                    &[],
                )
                .await?;

                info!("Fact registry mock address: {:?}", fact_registry_address);
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
            SettlementChain::Custom(_) => {
                // Safe to unwrap: validated in validate()
                JsonRpcClient::new(HttpTransport::new(self.settlement_rpc_url.clone().unwrap()))
            }
        }
    }
    pub fn get_fact_registry_address(&self, fact_registry_address: Option<Felt>) -> Felt {
        if let Some(addr) = fact_registry_address {
            return addr;
        }

        match &self.settlement_chain_id {
            SettlementChain::Mainnet => ATLANTIC_FACT_REGISTRY_MAINNET,
            SettlementChain::Sepolia => ATLANTIC_FACT_REGISTRY_SEPOLIA,
            SettlementChain::Custom(_) => {
                // Safe to unwrap: validated in validate() for SetupProgram command
                fact_registry_address.unwrap()
            }
        }
    }
}
