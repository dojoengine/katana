use anyhow::Result;
use clap::Subcommand;

use super::client::Client;

#[derive(Debug, Subcommand)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub enum KatanaCommands {
    /// Get the embedded settlement service status [katana_settlementStatus]
    SettlementStatus,
}

impl KatanaCommands {
    pub async fn execute(self, client: &Client) -> Result<()> {
        match self {
            KatanaCommands::SettlementStatus => {
                let result = client.katana_settlement_status().await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }
        }

        Ok(())
    }
}
