use clap::Args;
use starknet::core::types::Felt;

#[derive(Debug, Args, Clone)]
pub struct SettlementOptions {
    #[arg(help = "The settlement contract address.")]
    #[arg(long)]
    pub settlement_contract: Option<Felt>,
}
