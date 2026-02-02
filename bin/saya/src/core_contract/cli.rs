use std::{fs, path::Path};

use anyhow::Result;
use clap::{Args, Parser, Subcommand};

use anyhow::anyhow;
use cairo_lang_starknet_classes::casm_contract_class::CasmContractClass;
use cairo_lang_starknet_classes::contract_class::ContractClass;
use dojo_utils::{Declarer, Deployer, Invoker, LabeledClass, TransactionResult, TxnConfig};
use log::{info, warn};
use starknet::accounts::{Account, SingleOwnerAccount};
use starknet::core::types::contract::SierraClass;
use starknet::core::types::{Call, Felt, FlattenedSierraClass};
use starknet::core::utils::cairo_short_string_to_felt;
use starknet::macros::selector;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider};
use starknet::signers::{LocalWallet, SigningKey};
use starknet_api::contract_class::compiled_class_hash::{HashVersion, HashableCompiledClass};
use url::Url;

use crate::core_contract::constants::{
    ATLANTIC_FACT_REGISTRY_SEPOLIA, BOOTLOADER_PROGRAM_HASH, DEFAULT_PILTOVER_PATH,
    INITIAL_BLOCK_HASH, INITIAL_BLOCK_NUMBER, INITIAL_STATE_ROOT, LAYOUT_BRIDGE_PROGRAM_HASH,
    SEPOLIA_RPC_URL, SNOS_PROGRAM_HASH, STRK_FEE_TOKEN,
};
use crate::core_contract::short_string::compute_starknet_os_config_hash;

#[derive(Debug, Parser)]
pub struct CoreContract {
    #[clap(long, env = "SETTLEMENT_ACCOUNT_PRIVATE_KEY")]
    private_key: String,
    #[clap(long, env = "SETTLEMENT_ACCOUNT_ADDRESS")]
    account_address: String,
    #[clap(subcommand)]
    cmd: CoreContractCmd,
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
    #[clap(long, env = "FEE_TOKEN_ADDRESS",default_value = STRK_FEE_TOKEN.to_hex_string())]
    fee_token_address: Felt,
    #[clap(long, env = "CHAIN_ID")]
    chain_id: String,
}

#[derive(Debug, Args)]
pub struct DeployArgs {
    #[clap(long, env = "CLASS_HASH")]
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
        let provider = JsonRpcClient::new(HttpTransport::new(Url::parse(SEPOLIA_RPC_URL)?));
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
            CoreContractCmd::SetupProgram(setup_program_args) => {
                let chain_id = cairo_short_string_to_felt(&setup_program_args.chain_id)?;

                let snos_config_hash = compute_starknet_os_config_hash(chain_id, STRK_FEE_TOKEN);
                info!("Starknet OS config hash: {:?}", snos_config_hash);
                let tx_res = set_program_info(
                    account.clone(),
                    setup_program_args.core_contract_address,
                    snos_config_hash,
                )
                .await?;

                info!("Set program info transaction submitted: {:?}", tx_res);
                let tx_res = set_fact_registry(
                    account.clone(),
                    setup_program_args.core_contract_address,
                    setup_program_args
                        .fact_registry_address
                        .unwrap_or(ATLANTIC_FACT_REGISTRY_SEPOLIA),
                )
                .await?;
                info!("Fact registry set transaction submitted: {:?}", tx_res);
            }
        }

        Ok(())
    }
}

pub async fn declare_core_contract(
    account: SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>,
    core_contract_path: &Path,
) -> Result<Felt> {
    let txn_config = TxnConfig::default();

    let mut declarer = Declarer::new(account, txn_config);
    let class = prepare_class(core_contract_path, true)?;
    let labeled = LabeledClass {
        label: class.label.clone(),
        casm_class_hash: class.casm_class_hash,
        class: class.class.clone(),
    };
    declarer.add_class(labeled);
    let results = declarer.declare_all().await?;

    // There is only one class to declare.
    let class_hash = match &results[0] {
        TransactionResult::Noop => {
            info!("Core contract already declared on-chain.");
            class.class_hash
        }
        TransactionResult::Hash(hash) => {
            info!("Core contract declared.");
            info!("  Tx hash   : {hash:?}");
            *hash
        }
        TransactionResult::HashReceipt(hash, receipt) => {
            info!("Core contract declared.");
            info!("  Tx hash   : {hash:?}");
            info!(" Declared on block  : {:?}", receipt.block.block_number());
            *hash
        }
    };
    Ok(class_hash)
}

pub async fn deploy_core_contract(
    account: SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>,
    class_hash: Felt,
    salt: Felt,
) -> Result<Felt> {
    let txn_config = dojo_utils::TxnConfig {
        receipt: true,
        ..Default::default()
    };

    let deployer = Deployer::new(account.clone(), txn_config);
    let constructor_calldata: Vec<Felt> = vec![
        // owner.
        account.address(),
        // state root.
        INITIAL_STATE_ROOT,
        // block_number must be magic value for genesis block.
        INITIAL_BLOCK_NUMBER,
        // block_hash.
        INITIAL_BLOCK_HASH,
    ];

    match deployer
        .deploy_via_udc(class_hash, salt, &constructor_calldata, account.address())
        .await
    {
        Ok((contract_address, transaction_result)) => {
            info!("Core contract deployed.");
            match transaction_result {
                TransactionResult::Noop => {
                    info!("noop (already deployed).");
                }
                TransactionResult::Hash(hash) => {
                    info!(" Tx hash   : {hash:?}");
                }
                TransactionResult::HashReceipt(hash, receipt) => {
                    info!(" Tx hash   : {hash:?}");
                    info!(" Deployed on block  : {:?}", receipt.block.block_number());
                }
            }
            Ok(contract_address)
        }
        Err(e) => {
            let address = try_extract_already_deployed_address(&e)?;
            if let Some(address) = address {
                warn!("Core contract already deployed at address: {:?}", address);
                return Ok(address);
            }
            Err(anyhow!("Deployment failed: {:?}", e))
        }
    }
}

fn try_extract_already_deployed_address<E: std::fmt::Debug>(e: &E) -> anyhow::Result<Option<Felt>> {
    const MARKER: &str = "already deployed at address ";
    let msg = format!("{:?}", e);

    let i = match msg.find(MARKER) {
        Some(i) => i + MARKER.len(),
        None => return Ok(None),
    };

    let rest = &msg[i..];
    let end = rest
        .char_indices()
        .find(|&(_, c)| !(c.is_ascii_hexdigit() || c == 'x'))
        .map(|(j, _)| j)
        .unwrap_or(rest.len());

    let addr = &rest[..end];
    if !addr.starts_with("0x") {
        return Ok(None);
    }

    Ok(Some(Felt::from_hex(addr)?))
}

pub async fn set_program_info(
    account: SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>,
    contract_address: Felt,
    snos_config_hash: Felt,
) -> Result<TransactionResult> {
    let txn_config = TxnConfig::default();
    let invoker = Invoker::new(account, txn_config);

    let call = Call {
        to: contract_address,
        selector: selector!("set_program_info"),
        calldata: vec![
            BOOTLOADER_PROGRAM_HASH,
            snos_config_hash,
            SNOS_PROGRAM_HASH,
            LAYOUT_BRIDGE_PROGRAM_HASH,
        ],
    };
    let tx = invoker.invoke(call).await.unwrap();
    Ok(tx)
}

pub async fn set_fact_registry(
    account: SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>,
    contract_address: Felt,
    fact_registry_address: Felt,
) -> Result<TransactionResult> {
    let txn_config = TxnConfig::default();
    let invoker = Invoker::new(account, txn_config);

    let call = Call {
        to: contract_address,
        selector: selector!("set_facts_registry"),
        calldata: vec![fact_registry_address],
    };
    let tx = invoker.invoke(call).await.unwrap();
    Ok(tx)
}

#[derive(Debug, Clone)]
struct PreparedClass {
    label: String,
    class_hash: Felt,
    casm_class_hash: Felt,
    class: FlattenedSierraClass,
}

fn prepare_class(path: &Path, use_blake2s: bool) -> Result<PreparedClass> {
    let data = fs::read(path)?;

    let sierra: SierraClass = serde_json::from_slice(&data)?;
    let class_hash = sierra.class_hash()?;
    let flattened = sierra.clone().flatten()?;

    let casm_hash = casm_class_hash_from_bytes(&data, use_blake2s)?;

    let label = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("Unable to infer contract name from {}", path.display()))?
        .split('.')
        .next()
        .ok_or_else(|| anyhow!("Unable to infer contract name from {}", path.display()))?
        .to_string();

    Ok(PreparedClass {
        label,
        class_hash,
        casm_class_hash: casm_hash,
        class: flattened,
    })
}

fn casm_class_hash_from_bytes(data: &[u8], use_blake2s: bool) -> Result<Felt> {
    let sierra_class: ContractClass = serde_json::from_slice(data)?;
    let casm_class = CasmContractClass::from_contract_class(sierra_class, false, usize::MAX)?;

    let hash_version = if use_blake2s {
        HashVersion::V2
    } else {
        HashVersion::V1
    };
    let hash = casm_class.hash(&hash_version);

    Ok(Felt::from_bytes_be(&hash.0.to_bytes_be()))
}
