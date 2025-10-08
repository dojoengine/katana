use std::path::PathBuf;

use anyhow::{anyhow, Result};
pub use clap::Parser;
use serde::{Deserialize, Serialize};

use crate::options::*;

#[derive(Parser, Debug, Serialize, Deserialize, Default, Clone, PartialEq)]
#[command(next_help_heading = "Full node options")]
pub struct FullNodeArgs {
    /// Don't print anything on startup.
    #[arg(long)]
    pub silent: bool,

    /// Directory path of the database to initialize from.
    ///
    /// The path must either be an empty directory or a directory which already contains a
    /// previously initialized Katana database.
    #[arg(long)]
    #[arg(value_name = "PATH")]
    pub db_dir: Option<PathBuf>,

    /// Ethereum RPC URL for querying the Starknet Core Contract.
    #[arg(long)]
    #[arg(value_name = "URL")]
    pub eth_rpc_url: String,

    /// Starknet network to sync from (mainnet or sepolia).
    #[arg(long, default_value = "sepolia")]
    #[arg(value_parser = ["mainnet", "sepolia"])]
    pub network: String,

    #[command(flatten)]
    pub logging: LoggingOptions,

    #[command(flatten)]
    pub tracer: TracerOptions,

    #[cfg(feature = "server")]
    #[command(flatten)]
    pub metrics: MetricsOptions,

    #[cfg(feature = "server")]
    #[command(flatten)]
    pub server: ServerOptions,

    #[cfg(feature = "explorer")]
    #[command(flatten)]
    pub explorer: ExplorerOptions,
}

impl FullNodeArgs {
    pub async fn execute(&self) -> Result<()> {
        Err(anyhow!("Full node is not implemented yet!"))
    }
}
