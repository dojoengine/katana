use katana_primitives::receipt::Receipt;
use katana_primitives::transaction::TxNumber;

use super::{MigrationError, MigrationStage, BATCH_SIZE};
use crate::abstraction::{Database, DbCursor, DbTx, DbTxMut};
use crate::mdbx::tx::TxRW;
use crate::models::ReceiptEnvelope;
use crate::version::Version;
use crate::{tables, Db};

/// The database version below which receipts are stored as raw postcard bytes (legacy
/// `Receipt` codec) and need to be re-encoded into the [`ReceiptEnvelope`] format.
const RECEIPT_ENVELOPE_VERSION: Version = Version::new(9);

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
        "migration/receipt-envelope"
    }

    fn threshold_version(&self) -> Version {
        RECEIPT_ENVELOPE_VERSION
    }

    fn total_items(&self, db: &Db) -> Result<u64, MigrationError> {
        Ok(db.view(|tx| tx.entries::<LegacyReceipts>())? as u64)
    }

    fn process_batch(&self, tx: &TxRW, start_key: u64) -> Result<Option<u64>, MigrationError> {
        // Read a batch using the legacy Receipt codec.
        let batch = {
            let mut cursor = tx.cursor::<LegacyReceipts>()?;
            let walker = cursor.walk(Some(start_key))?;

            let mut entries = Vec::new();
            for result in walker {
                let (tx_number, receipt) = result?;
                entries.push((tx_number, receipt));
                if entries.len() as u64 >= BATCH_SIZE {
                    break;
                }
            }
            entries
        };

        let count = batch.len() as u64;
        let next_key =
            if count < BATCH_SIZE { None } else { batch.last().map(|(last_key, _)| last_key + 1) };

        // Write back as ReceiptEnvelope.
        for (tx_number, receipt) in batch {
            tx.put::<tables::Receipts>(tx_number, ReceiptEnvelope::from(receipt))?;
        }

        Ok(next_key)
    }
}
