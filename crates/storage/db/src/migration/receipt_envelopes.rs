use indicatif::{ProgressBar, ProgressStyle};
use katana_primitives::receipt::Receipt;
use katana_primitives::transaction::TxNumber;

use super::{MigrationError, MigrationStage, BATCH_SIZE};
use crate::abstraction::{Database, DbCursor, DbTx, DbTxMut};
use crate::models::stage::MigrationCheckpoint;
use crate::models::ReceiptEnvelope;
use crate::version::Version;
use crate::{tables, Db};

/// The database version below which receipts are stored as raw postcard bytes (legacy
/// `Receipt` codec) and need to be re-encoded into the [`ReceiptEnvelope`] format.
const RECEIPT_ENVELOPE_VERSION: Version = Version::new(9);

/// Stage ID used to checkpoint receipt envelope migration progress in the
/// [`MigrationCheckpoints`](tables::MigrationCheckpoints) table.
const RECEIPT_MIGRATION_STAGE: &str = "migration/receipt-envelope";

/// Shadow table definition that reads from the physical `Receipts` table using the legacy
/// `Receipt` (raw postcard) codec instead of `ReceiptEnvelope`.
#[derive(Debug)]
struct LegacyReceipts;

impl tables::Table for LegacyReceipts {
    const NAME: &'static str = tables::Receipts::NAME;
    type Key = TxNumber;
    type Value = Receipt;
}

pub(crate) struct ReceiptEnvelopeStage;

impl MigrationStage for ReceiptEnvelopeStage {
    fn id(&self) -> &'static str {
        RECEIPT_MIGRATION_STAGE
    }

    fn threshold_version(&self) -> Version {
        RECEIPT_ENVELOPE_VERSION
    }

    fn execute(&self, db: &Db) -> Result<(), MigrationError> {
        let total_entries = db.view(|tx| tx.entries::<LegacyReceipts>())? as u64;

        if total_entries == 0 {
            eprintln!(
                "[Migrating] \x1b[1;33m{RECEIPT_MIGRATION_STAGE}         \x1b[0m table is empty, \
                 nothing to migrate."
            );
            return Ok(());
        }

        // Resume from the last checkpoint if one exists.
        let checkpoint = db.view(|tx| {
            tx.get::<tables::MigrationCheckpoints>(RECEIPT_MIGRATION_STAGE.to_string())
        })?;

        let start_key = checkpoint.map(|cp| cp.next_key).unwrap_or(0);
        let already_migrated = start_key;

        if start_key >= total_entries {
            eprintln!("[Migrating] \x1b[1;33mReceipts         \x1b[0m already up to date.");
            // Clean up the checkpoint.
            db.update(|tx| {
                tx.delete::<tables::MigrationCheckpoints>(
                    RECEIPT_MIGRATION_STAGE.to_string(),
                    None,
                )?;
                Ok(())
            })?;
            return Ok(());
        }

        let remaining = total_entries - already_migrated;

        let pb = ProgressBar::new(remaining);
        pb.set_style(
            ProgressStyle::default_bar()
                .template(
                    "[Migrating] \x1b[1;33mReceipts         \x1b[0m {bar:40.cyan/blue} \
                     {pos}/{len} receipts [{elapsed_precise}] {per_sec}",
                )
                .expect("valid format"),
        );

        let mut migrated: u64 = 0;
        let mut batch_start_key: Option<u64> = Some(start_key);

        while let Some(start) = batch_start_key {
            // Phase 1: Read a batch using the legacy Receipt codec.
            let batch: Vec<(u64, Receipt)> = {
                let tx = db.tx()?;
                let mut cursor = tx.cursor::<LegacyReceipts>()?;
                let walker = cursor.walk(Some(start))?;

                let mut entries = Vec::new();
                for result in walker {
                    let (tx_number, receipt) = result?;
                    entries.push((tx_number, receipt));
                    if entries.len() as u64 >= BATCH_SIZE {
                        break;
                    }
                }

                tx.commit()?;
                entries
            };

            let current_batch_size = batch.len() as u64;
            let is_last_batch = current_batch_size < BATCH_SIZE;

            let next_key =
                if is_last_batch { None } else { batch.last().map(|(last_key, _)| last_key + 1) };

            // Phase 2: Write re-encoded receipts and checkpoint atomically.
            db.update(|tx| {
                for (tx_number, receipt) in batch {
                    tx.put::<tables::Receipts>(tx_number, ReceiptEnvelope::from(receipt))?;
                }

                if let Some(next) = next_key {
                    // Persist progress so we can resume from here on crash.
                    tx.put::<tables::MigrationCheckpoints>(
                        RECEIPT_MIGRATION_STAGE.to_string(),
                        MigrationCheckpoint { next_key: next },
                    )?;
                } else {
                    // Migration complete — remove the checkpoint.
                    tx.delete::<tables::MigrationCheckpoints>(
                        RECEIPT_MIGRATION_STAGE.to_string(),
                        None,
                    )?;
                }

                Ok(())
            })?;

            migrated += current_batch_size;
            pb.set_position(migrated);
            batch_start_key = next_key;
        }

        pb.finish_with_message(format!("[Migrating] Re-encoded {migrated} receipts"));

        Ok(())
    }
}
