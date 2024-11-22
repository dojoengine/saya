#![warn(unused_crate_dependencies)]

//! Saya executable entry point.
use clap::Parser;
use console::Style;
use saya_core::{Saya, SayaConfig};

mod args;

use args::SayaArgs;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = SayaArgs::parse();
    args.init_logging()?;
    let saya_config = args.try_into()?;
    print_intro(&saya_config);
    let mut saya = Saya::new(saya_config).await?;
    saya.start().await?;
    Ok(())
}

fn print_intro(config: &SayaConfig) {
    println!(
        "{}",
        Style::new().color256(94).apply_to(
            r"

 _______  _______           _______
(  ____ \(  ___  )|\     /|(  ___  )
| (    \/| (   ) |( \   / )| (   ) |
| (_____ | (___) | \ (_) / | (___) |
(_____  )|  ___  |  \   /  |  ___  |
      ) || (   ) |   ) (   | (   ) |
/\____) || )   ( |   | |   | )   ( |
\_______)|/     \|   \_/   |/     \|
"
        )
    );

    println!(
        r"
CONFIGURATION
=============
Settlement contract: {:#x}
Settlement account: {:#x}
Chain ID: {:#x},
RPC url: {:#},

    ",
        config.settlement_contract, config.starknet_account.signer_address,
        config.starknet_account.chain_id, config.rpc_url.as_str()
    );
    println!(
        r"
PROVER
==================
Prover: {}
    ",
        config.prover_url
    );

    println!(
        r"
VERIFIER
==================
    ",
    );
}
