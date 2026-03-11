use std::time::Instant;

use anyhow::{Context, Result};
use clap::Args;
use indicatif::{ProgressBar, ProgressStyle};
use inquire::Confirm;
use katana_db::abstraction::{Database, DbCursor, DbCursorMut, DbTx, DbTxMut};
use katana_db::tables;
use katana_db::version::DbOpenMode;
use katana_primitives::transaction::TxNumber;

use super::{open_db_ro, open_db_rw};

/// The default number of receipts to process per transaction batch.
const DEFAULT_BATCH_SIZE: usize = 10_000;

#[derive(Debug, Args)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct MigrateArgs {
    /// Path to the database directory.
    #[arg(short, long)]
    pub path: String,

    /// How Katana should open supported older database versions.
    #[arg(long = "db-open-mode")]
    #[arg(default_value_t = DbOpenMode::Compat)]
    #[arg(value_name = "MODE")]
    pub open_mode: DbOpenMode,

    /// Number of receipt rows to process per transaction batch.
    #[arg(long, default_value_t = DEFAULT_BATCH_SIZE)]
    pub batch_size: usize,

    /// Skip confirmation prompt.
    #[arg(short = 'y')]
    pub skip_confirmation: bool,
}

impl MigrateArgs {
    pub fn execute(self) -> Result<()> {
        // Count entries first using a read-only transaction.
        let total = {
            let db = open_db_ro(&self.path, self.open_mode)?;
            let tx = db.tx().context("Failed to create read transaction")?;
            let count = tx.entries::<tables::Receipts>()?;
            drop(tx);
            drop(db);
            count
        };

        if total == 0 {
            println!("Receipts table is empty. Nothing to migrate.");
            return Ok(());
        }

        println!("Found {total} receipt entries to migrate.");

        if !self.skip_confirmation {
            let ans = Confirm::new("Rewrite all Receipts rows with zstd compression?")
                .with_default(false)
                .with_help_message("Press Enter for default (No)")
                .prompt()?;

            if !ans {
                println!("Migration cancelled.");
                return Ok(());
            }
        }

        let start = Instant::now();

        let pb = ProgressBar::new(total as u64);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{msg} {bar:40.cyan/blue} {pos:>9}/{len:9} [{elapsed_precise}] {per_sec}")
                .unwrap()
                .progress_chars("##-"),
        );
        pb.set_message("Migrating receipts");

        let db = open_db_rw(&self.path, self.open_mode)?;
        let mut last_key: Option<TxNumber> = None;
        let mut total_migrated: u64 = 0;

        loop {
            let tx = db.tx_mut().context("Failed to create write transaction")?;
            let mut cursor = tx.cursor_mut::<tables::Receipts>()?;

            // Seek to the starting position for this batch.
            let first_entry = match last_key {
                Some(key) => {
                    // Seek past the last processed key.
                    cursor.seek(key + 1)?
                }
                None => cursor.first()?,
            };

            let Some((mut key, value)) = first_entry else {
                // No more entries to process.
                tx.commit().context("Failed to commit final transaction")?;
                break;
            };

            // Re-put the first entry of this batch.
            tx.put::<tables::Receipts>(key, value)?;
            total_migrated += 1;
            pb.inc(1);

            let mut batch_count = 1usize;

            // Process the rest of the batch using cursor.next().
            while batch_count < self.batch_size {
                let Some((next_key, next_value)) = cursor.next()? else {
                    break;
                };
                key = next_key;
                tx.put::<tables::Receipts>(key, next_value)?;
                total_migrated += 1;
                batch_count += 1;
                pb.inc(1);
            }

            last_key = Some(key);
            tx.commit().context("Failed to commit batch transaction")?;
        }

        pb.finish_with_message("Migration complete");

        let elapsed = start.elapsed();
        println!(
            "\nMigrated {total_migrated} receipts in {:.1}s ({:.0} rows/s)",
            elapsed.as_secs_f64(),
            total_migrated as f64 / elapsed.as_secs_f64(),
        );

        Ok(())
    }
}
