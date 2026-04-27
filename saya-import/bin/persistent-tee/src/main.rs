//! # persistent-tee
//!
//! Saya TEE proving orchestrator — runs the full TEE pipeline: block ingestion,
//! attestation, SP1 proof generation, and on-chain settlement.

use anyhow::Result;
use clap::{Parser, Subcommand};

mod attestor;
mod common;
mod mock_proof;
mod prover;
mod prover_impl;
mod settlement;
mod tee;

use tee::Tee;

#[derive(Debug, Parser)]
#[clap(about, version)]
struct Cli {
    #[clap(subcommand)]
    command: Subcommands,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Subcommand)]
enum Subcommands {
    /// Run Saya in TEE mode where blocks are proved inside a trusted execution
    /// environment and settled on-chain.
    Tee(Tee),
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    saya_tracing::init("info,persistent_tee=trace,saya_core=trace")?;

    match cli.command {
        Subcommands::Tee(cmd) => cmd.run().await,
    }
}
