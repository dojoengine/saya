//! Data availability options.

use clap::Args;
use starknet::core::types::Felt;
use url::Url;

#[derive(Debug, Args, Clone)]
pub struct StarknetAccountOptions {
    #[arg(long, env)]
    #[arg(help = "The url of the starknet node.")]
    pub starknet_url: Url,

    #[arg(long)]
    #[arg(help = "The chain id of the starknet node.")]
    #[arg(default_value = "SN_SEPOLIA")]
    pub chain_id: String,

    #[arg(long, env)]
    #[arg(help = "The address of the starknet account.")]
    pub signer_address: Felt,

    #[arg(long, env)]
    #[arg(help = "The private key of the starknet account.")]
    pub signer_key: Option<Felt>,

    #[arg(long = "keystore")]
    #[arg(value_name = "PATH")]
    #[arg(help = "The path to the keystore file.")]
    pub signer_keystore_path: Option<String>,

    #[arg(long = "password")]
    #[arg(value_name = "PASSWORD")]
    #[arg(help = "The password to the keystore file.")]
    pub signer_keystore_password: Option<String>,
}
