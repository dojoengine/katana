use std::ops::RangeInclusive;

use katana_primitives::receipt::Receipt;
use katana_primitives::transaction::TxNumber;

use super::{MigrationError, MigrationStage};
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

    fn range(&self, db: &Db) -> Result<Option<RangeInclusive<u64>>, MigrationError> {
        let total = db.view(|tx| tx.entries::<LegacyReceipts>())? as u64;
        if total == 0 {
            Ok(None)
        } else {
            Ok(Some(0..=total - 1))
        }
    }

    fn execute(&self, tx: &TxRW, range: RangeInclusive<u64>) -> Result<(), MigrationError> {
        // Read the batch using the legacy Receipt codec via cursor walk.
        let batch: Vec<(u64, Receipt)> = {
            let mut cursor = tx.cursor::<LegacyReceipts>()?;
            let walker = cursor.walk(Some(*range.start()))?;

            let mut entries = Vec::new();
            for result in walker {
                let (tx_number, receipt) = result?;
                if tx_number > *range.end() {
                    break;
                }
                entries.push((tx_number, receipt));
            }
            entries
        };

        // Write back as ReceiptEnvelope.
        for (tx_number, receipt) in batch {
            tx.put::<tables::Receipts>(tx_number, ReceiptEnvelope::from(receipt))?;
        }

        Ok(())
    }
}
