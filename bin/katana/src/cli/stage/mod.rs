use anyhow::Result;
use clap::{Args, Subcommand};

mod checkpoint;

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
}

impl StageArgs {
    pub fn execute(self) -> Result<()> {
        match self.commands {
            Commands::Checkpoint(args) => args.execute(),
        }
    }
}
