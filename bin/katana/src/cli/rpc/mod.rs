use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use url::Url;

mod client;
mod node;
mod starknet;
mod tee;
mod txpool;

#[derive(Debug, Args)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct RpcArgs {
    #[command(subcommand)]
    command: Commands,

    #[command(flatten)]
    server: ServerOptions,
}

#[derive(Debug, Subcommand)]
#[cfg_attr(test, derive(PartialEq, Eq))]
enum Commands {
    /// Starknet JSON-RPC methods
    Starknet {
        #[command(subcommand)]
        command: starknet::StarknetCommands,
    },
    /// Transaction pool JSON-RPC methods
    Txpool {
        #[command(subcommand)]
        command: txpool::TxpoolCommands,
    },
    /// Node JSON-RPC methods
    Node {
        #[command(subcommand)]
        command: node::NodeCommands,
    },
    /// TEE JSON-RPC methods
    Tee {
        #[command(subcommand)]
        command: tee::TeeCommands,
    },
}

impl RpcArgs {
    pub async fn execute(self) -> Result<()> {
        let client = self.client().context("Failed to create client")?;
        match self.command {
            Commands::Starknet { command } => command.execute(&client).await,
            Commands::Txpool { command } => command.execute(&client).await,
            Commands::Node { command } => command.execute(&client).await,
            Commands::Tee { command } => command.execute(&client).await,
        }
    }

    fn client(&self) -> Result<client::Client> {
        client::Client::new(Url::parse(&self.server.url)?)
    }
}

#[derive(Debug, Args)]
#[cfg_attr(test, derive(PartialEq, Eq))]
#[command(next_help_heading = "Server options")]
pub struct ServerOptions {
    /// Katana RPC endpoint URL
    #[arg(global = true)]
    #[arg(long, default_value = "http://localhost:5050")]
    url: String,
}
