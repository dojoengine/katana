#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use anyhow::Result;
use clap::{Args, Subcommand};

pub mod args;
pub mod file;
pub mod full;
pub mod optimistic;
pub mod options;
pub mod utils;

pub use args::SequencerNodeArgs;
pub use options::*;

use crate::full::FullNodeArgs;
use crate::optimistic::OptimisticNodeArgs;

#[derive(Debug, Args, PartialEq)]
pub struct NodeCli {
    #[command(subcommand)]
    pub command: NodeSubcommand,
}

#[derive(Debug, Subcommand, PartialEq)]
pub enum NodeSubcommand {
    #[command(about = "Launch a full node", hide = true)]
    Full(Box<FullNodeArgs>),

    #[command(about = "Launch a sequencer node")]
    Sequencer(Box<SequencerNodeArgs>),

    #[command(about = "Launch an optimistic node")]
    Optimistic(Box<OptimisticNodeArgs>),
}

impl NodeCli {
    pub async fn execute(self) -> Result<()> {
        match self.command {
            NodeSubcommand::Full(args) => args.execute().await,
            NodeSubcommand::Sequencer(args) => args.with_config_file()?.execute().await,
            NodeSubcommand::Optimistic(args) => args.execute().await,
        }
    }
}
