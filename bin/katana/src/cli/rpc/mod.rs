use anyhow::{Context, Result};
use clap::Args;
use url::Url;

mod client;
mod starknet;

#[derive(Debug, Args)]
pub struct RpcArgs {
    #[command(subcommand)]
    command: starknet::StarknetCommands,

    #[command(flatten)]
    server: ServerOptions,
}

impl RpcArgs {
    pub async fn execute(self) -> Result<()> {
        let client = self.client().context("Failed to create client")?;
        self.command.execute(&client).await
    }

    fn client(&self) -> Result<client::Client> {
        client::Client::new(Url::parse(&self.server.url)?)
    }
}

#[derive(Debug, Args)]
#[command(next_help_heading = "Server options")]
pub struct ServerOptions {
    /// Katana RPC endpoint URL
    #[arg(global = true)]
    #[arg(long, default_value = "http://localhost:5050")]
    url: String,
}
