//! # persistent-tee
//!
//! Saya TEE proving orchestrator — runs the full TEE pipeline: block ingestion,
//! attestation, SP1 proof generation, and on-chain settlement.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

mod attestor;
mod common;
mod mock_proof;
mod prover;
mod prover_impl;
mod settlement;
mod tee;

use katana_tracing::{EnvFilter, LogColor, LogFormat};
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
    initialize_logging().await?;

    match Cli::parse().command {
        Subcommands::Tee(cmd) => cmd.run().await,
    }
}

async fn initialize_logging() -> Result<()> {
    const DEFAULT_LOG_FILTER: &str = "info,persistent_tee=trace,saya_core=trace";

    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(DEFAULT_LOG_FILTER))
        .context("failed to parse log filter")?;

    let logging = katana_tracing::LoggingConfig {
        stdout_format: LogFormat::Full,
        stdout_color: LogColor::Always,
        ..Default::default()
    };

    Ok(katana_tracing::init(logging, None, filter).await?)
}
