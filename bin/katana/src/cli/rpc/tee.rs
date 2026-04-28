use anyhow::Result;
use clap::{Args, Subcommand};
use katana_primitives::block::BlockNumber;

use super::client::Client;

#[derive(Debug, Subcommand)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub enum TeeCommands {
    /// Generate a TEE attestation quote for a block's state [tee_generateQuote]
    #[command(name = "generate-quote")]
    GenerateQuote(GenerateQuoteArgs),

    /// Get the Merkle inclusion proof for an event [tee_getEventProof]
    #[command(name = "event-proof")]
    EventProof(EventProofArgs),
}

#[derive(Debug, Args)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct GenerateQuoteArgs {
    /// Block number to attest to
    #[arg(long)]
    block_id: BlockNumber,

    /// Previous block number to chain the attestation against
    #[arg(long)]
    prev_block_id: Option<BlockNumber>,
}

#[derive(Debug, Args)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct EventProofArgs {
    /// Block number containing the event
    #[arg(long)]
    block_number: BlockNumber,

    /// Index of the event within the block
    #[arg(long)]
    event_index: u32,
}

impl TeeCommands {
    pub async fn execute(self, client: &Client) -> Result<()> {
        match self {
            TeeCommands::GenerateQuote(args) => {
                let result = client.tee_generate_quote(args.prev_block_id, args.block_id).await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }

            TeeCommands::EventProof(args) => {
                let result =
                    client.tee_get_event_proof(args.block_number, args.event_index).await?;
                println!("{}", colored_json::to_colored_json_auto(&result)?);
            }
        }

        Ok(())
    }
}
