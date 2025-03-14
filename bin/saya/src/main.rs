//! # Saya
//!
//! Saya is the proving orchestrator of the Dojo stack. `saya` is a binary crate for a command line
//! application for running Saya.

use anyhow::Result;
use clap::{Parser, Subcommand};

mod sovereign;
use sovereign::Sovereign;

mod persistent;
use persistent::Persistent;

mod sharding;
use sharding::Sharding;

mod any;

mod common;

#[derive(Debug, Parser)]
#[clap(about, version)]
struct Cli {
    #[clap(subcommand)]
    command: Subcommands,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Subcommand)]
enum Subcommands {
    /// Run and manage Saya in sovereign mode where the network settles interally without a "base
    /// layer".
    Sovereign(Sovereign),
    /// Run and manage Saaya in persistent L3 mode where proofs are settled in a "base layer"
    /// network.
    Persistent(Persistent),
    Sharding(Sharding),
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var(
            "RUST_LOG",
            "info,saya=trace,saya_core=trace,rpc_client=info,prove_block=info",
        );
    }
    env_logger::init();

    match cli.command {
        Subcommands::Sovereign(cmd) => cmd.run().await,
        Subcommands::Persistent(cmd) => cmd.run().await,
        Subcommands::Sharding(cmd) => cmd.run().await,
    }
}
