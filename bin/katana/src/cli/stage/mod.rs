use anyhow::Result;
use clap::{Args, Subcommand};

use crate::cli::execute_async;

mod checkpoint;
mod unwind;

#[derive(Debug, Args)]
#[cfg_attr(test, derive(PartialEq))]
pub struct StageArgs {
    #[command(subcommand)]
    commands: Commands,
}

#[derive(Debug, Subcommand)]
#[cfg_attr(test, derive(PartialEq))]
enum Commands {
    /// Manage stage checkpoints
    Checkpoint(checkpoint::CheckpointArgs),
    /// Unwind a stage to a previous state
    Unwind(unwind::UnwindArgs),
}

impl StageArgs {
    pub fn execute(self) -> Result<()> {
        match self.commands {
            Commands::Checkpoint(args) => args.execute(),
            Commands::Unwind(args) => execute_async(args.execute())?,
        }
    }
}
