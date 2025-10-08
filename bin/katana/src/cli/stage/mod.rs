use anyhow::Result;
use clap::{Args, Subcommand};

mod checkpoint;

#[derive(Debug, Args)]
pub struct StageArgs {
    #[command(subcommand)]
    commands: Commands,
}

#[derive(Debug, Subcommand)]
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
