//! # Saya
//!
//! Saya is the proving orchestrator of the Dojo stack. `saya` is a binary crate for a command line
//! application for running Saya.

use anyhow::Result;
use clap::{Parser, Subcommand};

mod core_contract;
use core_contract::CoreContract;

mod celestia;
use celestia::Celestia;

#[derive(Debug, Parser)]
#[clap(about, version)]
struct Cli {
    #[clap(subcommand)]
    command: Subcommands,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Subcommand)]
enum Subcommands {
    /// Core contract utilities for deployment and management.
    CoreContract(CoreContract),
    /// Celestia utilities for namespace conversion and blob retrieval.
    Celestia(Celestia),
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    saya_tracing::init("info,saya=trace,saya_core=trace,rpc_client=info")?;
    match cli.command {
        Subcommands::CoreContract(cmd) => cmd.run().await,
        Subcommands::Celestia(cmd) => cmd.run().await,
    }
}
