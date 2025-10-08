use anyhow::Result;
use clap::{Args, Subcommand};
use katana_db::abstraction::DbTxMut;
use katana_db::models::stage::StageCheckpoint;
use katana_db::tables;
use katana_primitives::block::BlockNumber;

use crate::cli::db;

#[derive(Debug, Args)]
pub struct CheckpointArgs {
    #[command(subcommand)]
    commands: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Get the checkpoint block number for a stage
    Get(GetArgs),

    /// Set the checkpoint block number for a stage
    Set(SetArgs),
}

#[derive(Debug, Args)]
struct GetArgs {
    /// The stage ID to get checkpoint for
    #[arg(value_name = "STAGE_ID")]
    stage_id: String,

    /// Path to the database directory
    #[arg(short, long, default_value = "~/.katana/db")]
    db_path: String,
}

#[derive(Debug, Args)]
struct SetArgs {
    /// The stage ID to set checkpoint for
    #[arg(value_name = "STAGE_ID")]
    stage_id: String,

    /// The block number to set as checkpoint
    #[arg(value_name = "BLOCK_NUMBER")]
    block_number: BlockNumber,

    /// Path to the database directory
    #[arg(short, long, default_value = "~/.katana/db")]
    db_path: String,
}

impl CheckpointArgs {
    pub fn execute(self) -> Result<()> {
        match self.commands {
            Commands::Get(args) => args.execute(),
            Commands::Set(args) => args.execute(),
        }
    }
}

impl GetArgs {
    fn execute(self) -> Result<()> {
        let db = db::open_db_ro(&self.db_path)?;
        let tx = db.tx()?;

        match tx.get::<tables::StageCheckpoints>(self.stage_id.clone())? {
            Some(checkpoint) => {
                println!("stage '{}' checkpoint: {}", self.stage_id, checkpoint.block);
            }
            None => {
                println!("stage '{}' has no checkpoint set", self.stage_id);
            }
        }

        tx.commit()?;
        Ok(())
    }
}

impl SetArgs {
    fn execute(self) -> Result<()> {
        let db = db::open_db_rw(&self.db_path)?;
        let tx = db.tx_mut()?;

        let checkpoint = StageCheckpoint { block: self.block_number };
        tx.put::<tables::StageCheckpoints>(self.stage_id.clone(), checkpoint)?;

        tx.commit()?;

        println!("set checkpoint for stage '{}' to block {}", self.stage_id, self.block_number);

        Ok(())
    }
}
