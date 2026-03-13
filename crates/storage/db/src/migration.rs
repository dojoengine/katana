use std::collections::BTreeMap;

use indicatif::{ProgressBar, ProgressStyle};
use katana_primitives::block::BlockNumber;
use katana_primitives::contract::{StorageKey, StorageValue};
use katana_primitives::receipt::Receipt;
use katana_primitives::state::StateUpdates;
use katana_primitives::transaction::TxNumber;

use crate::abstraction::{Database, DbCursor, DbDupSortCursor, DbTx, DbTxMut};
use crate::error::DatabaseError;
use crate::models::class::MigratedCompiledClassHash;
use crate::models::contract::{ContractClassChange, ContractClassChangeType};
use crate::models::storage::ContractStorageEntry;
use crate::models::ReceiptEnvelope;
use crate::version::{self, Version, LATEST_DB_VERSION};
use crate::{tables, Db};

/// Errors that can occur during database migration.
#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    /// A database operation failed during migration.
    #[error("database error: {0}")]
    Database(#[from] DatabaseError),

    /// Reconstructing state updates for a specific block failed.
    #[error("failed to reconstruct state update for block {block}")]
    FailedToReconstructStateUpdate {
        /// The block number for which state update reconstruction failed.
        block: BlockNumber,

        #[source]
        source: DatabaseError,
    },

    /// Failed to update the database version file after migration.
    #[error("failed to update database version: {0}")]
    VersionUpdate(#[from] version::DatabaseVersionError),
}

const BATCH_SIZE: u64 = 1000;

/// The database version that introduced the `BlockStateUpdates` table.
/// Databases opened with a version below this need to perform state updates migration.
///
/// The schema changes as well as the version bump were introduced in this PR: <https://github.com/dojoengine/katana/pull/470>
const STATE_UPDATES_TABLE_VERSION: Version = Version::new(9);

/// The database version below which receipts are stored as raw postcard bytes (legacy
/// `Receipt` codec) and need to be re-encoded into the [`ReceiptEnvelope`] format.
const RECEIPT_ENVELOPE_VERSION: Version = Version::new(9);

/// Runs all applicable database migrations based on the on-disk version at open time.
///
/// Each migration path has its own version threshold. Only migrations whose version
/// is above the database's opened version will be executed.
#[derive(Debug)]
pub struct Migration<'a> {
    db: &'a Db,
}

impl<'a> Migration<'a> {
    pub fn new(db: &'a Db) -> Self {
        Self { db }
    }

    /// Returns `true` if any migration path needs to be run.
    pub fn is_needed(&self) -> bool {
        self.needs_state_update_backfill() || self.needs_receipt_envelope_backfill()
    }

    /// Runs the migration process.
    pub fn run(&self) -> Result<(), MigrationError> {
        eprintln!("[Migrating] Starting migration");

        if self.needs_state_update_backfill() {
            self.backfill_state_updates()?;
        }

        if self.needs_receipt_envelope_backfill() {
            self.backfill_receipt_envelopes()?;
        }

        // Update the on-disk version file to the latest version.
        version::write_db_version_file(self.db.path(), LATEST_DB_VERSION)?;

        eprintln!(
            "[Migrating] Migration complete (version updated to \x1b[1m{LATEST_DB_VERSION}\x1b[0m)"
        );

        Ok(())
    }

    /// The [`BlockStateUpdates`](tables::BlockStateUpdates) table was introduced in version 9.
    /// Databases opened with a version below this need the table backfilled from the legacy
    /// index tables.
    pub(crate) fn needs_state_update_backfill(&self) -> bool {
        self.db.version() < STATE_UPDATES_TABLE_VERSION
    }

    /// Databases opened with a version below [`RECEIPT_ENVELOPE_VERSION`] may contain
    /// receipts stored as raw postcard bytes (the legacy `Receipt` codec). These need to be
    /// re-encoded into the [`ReceiptEnvelope`] format.
    pub(crate) fn needs_receipt_envelope_backfill(&self) -> bool {
        self.db.version() < RECEIPT_ENVELOPE_VERSION
    }

    /// Backfills the `BlockStateUpdates` table by reconstructing state updates from the legacy
    /// index tables (`NonceChangeHistory`, `ClassChangeHistory`, `ClassDeclarations`,
    /// `CompiledClassHashes`, `MigratedCompiledClassHashes`, `StorageChangeHistory`).
    ///
    /// Processes blocks in batches for crash recovery — on restart, already-committed batches
    /// are skipped automatically.
    pub(crate) fn backfill_state_updates(&self) -> Result<(), MigrationError> {
        let db = self.db;

        // Determine latest block number from BlockHashes cursor
        let latest_block_number = {
            let last = db.view(|tx| tx.cursor::<tables::BlockHashes>()?.last())?;

            match last {
                Some((block_num, _)) => block_num,
                None => return Ok(()), // no blocks, nothing to migrate
            }
        };

        // Determine resume point: how many entries already exist in BlockStateUpdates
        let existing_count = db.view(|tx| tx.entries::<tables::BlockStateUpdates>())? as u64; // view returns Result<usize>

        let start_block = existing_count;
        let total_blocks = latest_block_number + 1;

        if start_block >= total_blocks {
            eprintln!("[Migrating] \x1b[1;33mBlockStateUpdates\x1b[0m already up to date.");
            return Ok(());
        }

        let blocks_to_migrate = total_blocks - start_block;

        let pb = ProgressBar::new(blocks_to_migrate);
        pb.set_style(
            ProgressStyle::default_bar()
                .template(
                    "[Migrating] \x1b[1;33mBlockStateUpdates\x1b[0m {bar:40.cyan/blue} \
                     {pos}/{len} blocks   [{elapsed_precise}] {per_sec}",
                )
                .expect("valid format"),
        );

        // Process in batches
        let mut batch_start = start_block;
        while batch_start <= latest_block_number {
            let batch_end = std::cmp::min(batch_start + BATCH_SIZE - 1, latest_block_number);

            let tx = db.tx_mut()?;
            for block in batch_start..=batch_end {
                let state_updates = reconstruct_state_update(&tx, block).map_err(|source| {
                    MigrationError::FailedToReconstructStateUpdate { block, source }
                })?;
                tx.put::<tables::BlockStateUpdates>(block, state_updates)?;
            }
            tx.commit()?;

            pb.set_position(batch_end - start_block + 1);

            batch_start = batch_end + 1;
        }

        pb.finish_with_message(format!("[Migrating] Migrated {blocks_to_migrate} blocks"));

        Ok(())
    }

    /// Re-encodes every row in the `Receipts` table from legacy raw-postcard bytes into the
    /// [`ReceiptEnvelope`] format.
    ///
    /// Reads rows through [`LegacyReceipts`] (the old `Receipt` postcard codec) and writes
    /// them back through [`tables::Receipts`] (the new `ReceiptEnvelope` codec).
    ///
    /// Processes entries in batches for crash recovery.
    pub(crate) fn backfill_receipt_envelopes(&self) -> Result<(), MigrationError> {
        /// Shadow table definition that reads from the physical `Receipts` table using the legacy
        /// `Receipt` (raw postcard) codec instead of `ReceiptEnvelope`.
        #[derive(Debug)]
        struct LegacyReceipts;

        impl tables::Table for LegacyReceipts {
            const NAME: &'static str = tables::Receipts::NAME;
            type Key = TxNumber;
            type Value = Receipt;
        }

        let db = self.db;

        let total_entries = db.view(|tx| tx.entries::<LegacyReceipts>())? as u64;

        if total_entries == 0 {
            eprintln!(
                "[Migrating] \x1b[1;33mReceipts         \x1b[0m table is empty, nothing to \
                 migrate."
            );
            return Ok(());
        }

        let pb = ProgressBar::new(total_entries);
        pb.set_style(
            ProgressStyle::default_bar()
                .template(
                    "[Migrating] \x1b[1;33mReceipts         \x1b[0m {bar:40.cyan/blue} \
                     {pos}/{len} receipts [{elapsed_precise}] {per_sec}",
                )
                .expect("valid format"),
        );

        let mut migrated: u64 = 0;
        let mut batch_start_key: Option<u64> = Some(0);

        while let Some(start_key) = batch_start_key {
            // Phase 1: Read a batch using the legacy Receipt codec.
            let batch: Vec<(u64, Receipt)> = {
                let tx = db.tx()?;
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

                tx.commit()?;
                entries
            };

            let current_batch_size = batch.len() as u64;
            let is_last_batch = current_batch_size < BATCH_SIZE;

            if is_last_batch {
                batch_start_key = None;
            } else if let Some((last_key, _)) = batch.last() {
                batch_start_key = Some(last_key + 1);
            } else {
                batch_start_key = None;
            }

            // Phase 2: Write back as ReceiptEnvelope.
            db.update(|tx| {
                for (tx_number, receipt) in batch {
                    tx.put::<tables::Receipts>(tx_number, ReceiptEnvelope::from(receipt))?;
                }
                Ok(())
            })?;

            migrated += current_batch_size;
            pb.set_position(migrated);
        }

        pb.finish_with_message(format!("[Migrating] Re-encoded {migrated} receipts"));

        Ok(())
    }
}

/// Collects all DupSort entries for a given primary key from a DupSort table.
fn dup_entries<Tx, T>(tx: &Tx, key: T::Key) -> Result<Vec<T::Value>, DatabaseError>
where
    Tx: DbTx,
    T: tables::DupSort,
{
    let mut cursor = tx.cursor_dup::<T>()?;
    let mut entries = Vec::new();

    if let Some(walker) = cursor.walk_dup(Some(key), None)? {
        for result in walker {
            let (_, value) = result?;
            entries.push(value);
        }
    }

    Ok(entries)
}

/// Reconstructs a [`StateUpdates`] for a single block from the legacy index tables (database
/// version < 9).
fn reconstruct_state_update<Tx: DbTx>(
    tx: &Tx,
    block_number: BlockNumber,
) -> Result<StateUpdates, DatabaseError> {
    let mut state_updates = StateUpdates::default();

    // --- Nonce updates ---
    for nonce_change in dup_entries::<Tx, tables::NonceChangeHistory>(tx, block_number)? {
        state_updates.nonce_updates.insert(nonce_change.contract_address, nonce_change.nonce);
    }

    // --- Class changes (deployed contracts + replaced classes) ---
    for class_change in dup_entries::<Tx, tables::ClassChangeHistory>(tx, block_number)? {
        let ContractClassChange { r#type, contract_address, class_hash } = class_change;

        match r#type {
            ContractClassChangeType::Deployed => {
                state_updates.deployed_contracts.insert(contract_address, class_hash);
            }
            ContractClassChangeType::Replaced => {
                state_updates.replaced_classes.insert(contract_address, class_hash);
            }
        }
    }

    // --- Class declarations ---
    for class_hash in dup_entries::<Tx, tables::ClassDeclarations>(tx, block_number)? {
        // Look up the compiled class hash. If it exists, it's a Sierra class declaration.
        // If not, it's a deprecated (Cairo 0) class declaration.
        if let Some(compiled_class_hash) = tx.get::<tables::CompiledClassHashes>(class_hash)? {
            state_updates.declared_classes.insert(class_hash, compiled_class_hash);
        } else {
            state_updates.deprecated_declared_classes.insert(class_hash);
        }
    }

    // --- Migrated compiled class hashes ---
    for migrated in dup_entries::<Tx, tables::MigratedCompiledClassHashes>(tx, block_number)? {
        let MigratedCompiledClassHash { class_hash, compiled_class_hash } = migrated;
        state_updates.migrated_compiled_classes.insert(class_hash, compiled_class_hash);
    }

    // --- Storage updates ---
    {
        let mut storage_map: BTreeMap<_, BTreeMap<StorageKey, StorageValue>> = BTreeMap::new();
        for entry in dup_entries::<Tx, tables::StorageChangeHistory>(tx, block_number)? {
            let ContractStorageEntry { key, value } = entry;
            storage_map.entry(key.contract_address).or_default().insert(key.key, value);
        }
        state_updates.storage_updates = storage_map;
    }

    Ok(state_updates)
}
