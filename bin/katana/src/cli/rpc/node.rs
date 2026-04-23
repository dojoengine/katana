use anyhow::Result;
use clap::Subcommand;

use super::client::Client;

#[derive(Debug, Subcommand)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub enum NodeCommands {
    /// Get node identity and build information
    Info,
}

impl NodeCommands {
    pub async fn execute(self, client: &Client) -> Result<()> {
        match self {
            NodeCommands::Info => {
                let result = client.node_get_info().await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }
        }

        Ok(())
    }
}
