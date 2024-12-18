use clap::Args;
use starknet::core::types::Felt;
use url::Url;

#[derive(Debug, Args, Clone)]
pub struct SettlementOptions {
    #[arg(help = "The settlement contract address.")]
    #[arg(long)]
    pub settlement_contract: Option<Felt>,

    #[arg(long)]
    #[arg(value_name = "SETTLEMENT RPC URL")]
    #[arg(help = "RPC URL to post the proofs and state updates to.")]
    #[arg(default_value = "https://api.cartridge.gg/x/starknet/sepolia")]
    pub settlement_rpc_url: Url,
}
