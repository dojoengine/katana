use anyhow::Result;
use clap::{Args, Subcommand};
use katana_primitives::ContractAddress;

use super::client::Client;

#[derive(Debug, Subcommand)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub enum TxpoolCommands {
    /// Get pending and queued transaction counts
    Status,

    /// Get all transactions in the pool, grouped by sender and nonce
    Content,

    /// Get pool contents filtered by sender address
    #[command(name = "content-from")]
    ContentFrom(AddressArgs),

    /// Get a human-readable summary of the pool
    Inspect,
}

#[derive(Debug, Args)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct AddressArgs {
    /// Sender address
    #[arg(value_name = "ADDRESS")]
    address: ContractAddress,
}

impl TxpoolCommands {
    pub async fn execute(self, client: &Client) -> Result<()> {
        match self {
            TxpoolCommands::Status => {
                let result = client.txpool_status().await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }

            TxpoolCommands::Content => {
                let result = client.txpool_content().await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }

            TxpoolCommands::ContentFrom(args) => {
                let result = client.txpool_content_from(args.address).await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }

            TxpoolCommands::Inspect => {
                let result = client.txpool_inspect().await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }
        }

        Ok(())
    }
}
