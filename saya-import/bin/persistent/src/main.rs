//! # Saya
//!
//! Saya is the proving orchestrator of the Dojo stack. `saya` is a binary crate for a command line
//! application for running Saya.

use anyhow::Result;
use clap::{Parser, Subcommand};

mod atlantic;
mod error;
mod mock;
mod orchestrator;
mod settlement;
mod utils;

mod sovereign;
use sovereign::Sovereign;

mod persistent;
use persistent::Start;

mod any;

mod common;

mod snos_pie_generator;

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
    /// Start Saya in persistent L3 mode where proofs are settled in a "base layer" network.
    Start(Start),
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    saya_tracing::init(
        "info,persistent=trace,saya_core=trace,rpc_client=info,prove_block=info,blockifier=off,\
         generate_pie=off,rpc_client=off,starknet_os=off",
    )?;
    match cli.command {
        Subcommands::Sovereign(cmd) => cmd.run().await,
        Subcommands::Start(cmd) => cmd.run().await,
    }
}
