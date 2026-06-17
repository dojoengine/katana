use anyhow::Result;
use clap::Subcommand;

use super::client::Client;

#[derive(Debug, Subcommand)]
#[cfg_attr(test, derive(PartialEq, Eq))]
// The single variant name matches the `katana_settlementStatus` method it maps to; more
// katana-namespace methods can be added as siblings later.
#[allow(clippy::enum_variant_names)]
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
