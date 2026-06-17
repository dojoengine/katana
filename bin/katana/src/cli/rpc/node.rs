use anyhow::Result;
use clap::Subcommand;

use super::client::Client;

#[derive(Debug, Subcommand)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub enum NodeCommands {
    /// Get node identity and build information [node_getInfo]
    Info,
    /// Get the node's runtime configuration [node_getConfig]
    Config,
}

impl NodeCommands {
    pub async fn execute(self, client: &Client) -> Result<()> {
        match self {
            NodeCommands::Info => {
                let result = client.node_get_info().await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }
            NodeCommands::Config => {
                let result = client.node_get_config().await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }
        }

        Ok(())
    }
}
