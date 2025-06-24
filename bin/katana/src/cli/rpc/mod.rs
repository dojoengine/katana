use anyhow::{Context, Result};
use clap::Args;

mod client;
mod starknet;

use client::Client;
use url::Url;

#[derive(Debug, Args)]
pub struct RpcArgs {
    /// Katana RPC endpoint URL
    #[arg(global = true)]
    #[arg(long, default_value = "http://localhost:5050")]
    url: String,

    #[command(subcommand)]
    command: starknet::StarknetCommands,
}

impl RpcArgs {
    pub async fn execute(self) -> Result<()> {
        let client = self.client().context("Failed to create client")?;
        match self.command {
            starknet::StarknetCommands::Read(args) => args.execute(&client).await,
            starknet::StarknetCommands::Trace(args) => args.execute(&client).await,
        }
    }

    fn client(&self) -> Result<Client> {
        Ok(Client::new(Url::parse(&self.url)?))
    }
}
