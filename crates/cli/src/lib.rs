#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use anyhow::Result;
use clap::{Args, Subcommand};

pub mod file;
pub mod full;
pub mod options;
pub mod sequencer;
#[cfg(feature = "paymaster")]
pub mod sidecar;
pub mod utils;

pub use options::*;

#[derive(Debug, Args, PartialEq)]
pub struct NodeCli {
    #[command(subcommand)]
    pub command: NodeSubcommand,
}

#[derive(Debug, Subcommand, PartialEq)]
pub enum NodeSubcommand {
    #[command(about = "Launch a full node", hide = true)]
    Full(Box<full::FullNodeArgs>),

    #[command(about = "Launch a sequencer node")]
    Sequencer(Box<sequencer::SequencerNodeArgs>),
}

impl NodeCli {
    pub async fn execute(self) -> Result<()> {
        match self.command {
            NodeSubcommand::Full(args) => args.execute().await,
            NodeSubcommand::Sequencer(args) => args.with_config_file()?.execute().await,
        }
    }
}
