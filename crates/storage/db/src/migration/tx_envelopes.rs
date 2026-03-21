use std::ops::RangeInclusive;

use katana_primitives::transaction::TxNumber;

use super::{MigrationError, MigrationStage};
use crate::abstraction::{Database, DbCursor, DbTx, DbTxMut};
use crate::mdbx::tx::TxRW;
use crate::models::versioned::transaction::{TxEnvelope, VersionedTx};
use crate::version::Version;
use crate::{tables, Db};

/// The database version below which transactions are stored as raw postcard bytes (legacy
/// `VersionedTx` codec) and need to be re-encoded into the [`TxEnvelope`] format.
const TX_ENVELOPE_VERSION: Version = Version::new(9);

/// Shadow table definition that reads from the physical `Transactions` table using the legacy
/// `VersionedTx` (raw postcard) codec instead of `TxEnvelope`.
///
/// NOTE: The `Transactions` table has been moved to static files in v10+. This shadow table
/// definition is only used during migration from v5-v8 databases.
#[derive(Debug)]
struct LegacyTransactions;

impl tables::Table for LegacyTransactions {
    const NAME: &'static str = "Transactions";
    type Key = TxNumber;
    type Value = VersionedTx;
}

/// Shadow table for writing TxEnvelope during migration.
#[derive(Debug)]
struct MigrationTransactions;

impl tables::Table for MigrationTransactions {
    const NAME: &'static str = "Transactions";
    type Key = TxNumber;
    type Value = TxEnvelope;
}

pub(crate) struct TxEnvelopeStage;

impl MigrationStage for TxEnvelopeStage {
    fn id(&self) -> &'static str {
        "migration/tx-envelope"
    }

    fn threshold_version(&self) -> Version {
        TX_ENVELOPE_VERSION
    }

    fn range(&self, db: &Db) -> Result<Option<RangeInclusive<u64>>, MigrationError> {
        let total = db.view(|tx| tx.entries::<LegacyTransactions>())? as u64;
        if total == 0 {
            Ok(None)
        } else {
            Ok(Some(0..=total - 1))
        }
    }

    fn execute(&self, tx: &TxRW, range: RangeInclusive<u64>) -> Result<(), MigrationError> {
        // Read the batch using the legacy VersionedTx codec via cursor walk.
        let batch: Vec<(u64, VersionedTx)> = {
            let mut cursor = tx.cursor::<LegacyTransactions>()?;
            let walker = cursor.walk(Some(*range.start()))?;

            let mut entries = Vec::new();
            for result in walker {
                let (tx_number, versioned_tx) = result?;
                if tx_number > *range.end() {
                    break;
                }
                entries.push((tx_number, versioned_tx));
            }
            entries
        };

        // Write back as TxEnvelope.
        for (tx_number, versioned_tx) in batch {
            tx.put::<MigrationTransactions>(tx_number, TxEnvelope::from(versioned_tx))?;
        }

        Ok(())
    }
}
