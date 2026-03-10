//! # Saya
//!
//! Saya is the proving orchestrator of the Dojo stack. `saya` is a binary crate for a command line
//! application for running Saya in TEE mode.

use anyhow::Result;
use clap::{Parser, Subcommand};

mod tee;
use tee::Tee;

mod core_contract;
use core_contract::CoreContract;

mod celestia;
use celestia::Celestia;

mod attestor;
mod common;
mod prover;
mod prover_impl;
mod settlement;

#[derive(Debug, Parser)]
#[clap(about, version)]
struct Cli {
    #[clap(subcommand)]
    command: Subcommands,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Subcommand)]
enum Subcommands {
    /// Run and manage Saya in TEE mode where blocks are proved inside a trusted execution
    /// environment and settled in a "base layer" network.
    Tee(Tee),
    /// Core contract utilities for deployment and management.
    CoreContract(CoreContract),
    /// Celestia utilities for namespace conversion and blob retrieval.
    Celestia(Celestia),
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "info,saya=trace,saya_core=trace");
    }
    env_logger::init();

    match cli.command {
        Subcommands::Tee(cmd) => cmd.run().await,
        Subcommands::CoreContract(cmd) => cmd.run().await,
        Subcommands::Celestia(cmd) => cmd.run().await,
    }
}
