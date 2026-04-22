#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use anyhow::Result;
use clap::{Args, Subcommand};

pub mod args;
pub mod file;
pub mod full;
pub mod options;
pub mod sidecar;
pub mod utils;

pub use args::SequencerNodeArgs;
pub use katana_node_config::build_info::BuildInfo;
pub use options::*;

use crate::full::FullNodeArgs;

#[derive(Debug, Args, PartialEq)]
pub struct NodeCli {
    #[command(subcommand)]
    pub command: NodeSubcommand,
}

#[derive(Debug, Subcommand, PartialEq)]
pub enum NodeSubcommand {
    #[command(about = "Launch a full node")]
    Full(Box<FullNodeArgs>),

    #[command(about = "Launch a sequencer node")]
    Sequencer(Box<SequencerNodeArgs>),
}

impl NodeCli {
    pub async fn execute(self, build_info: BuildInfo) -> Result<()> {
        match self.command {
            NodeSubcommand::Full(args) => args.execute(build_info).await,
            NodeSubcommand::Sequencer(args) => args.with_config_file()?.execute(build_info).await,
        }
    }
}
