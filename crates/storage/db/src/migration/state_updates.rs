use std::collections::BTreeMap;

use indicatif::{ProgressBar, ProgressStyle};
use katana_primitives::block::BlockNumber;
use katana_primitives::contract::{StorageKey, StorageValue};
use katana_primitives::state::StateUpdates;

use super::{MigrationError, MigrationStage, BATCH_SIZE};
use crate::abstraction::{Database, DbCursor, DbDupSortCursor, DbTx, DbTxMut};
use crate::error::DatabaseError;
use crate::models::class::MigratedCompiledClassHash;
use crate::models::contract::{ContractClassChange, ContractClassChangeType};
use crate::models::stage::MigrationCheckpoint;
use crate::models::storage::ContractStorageEntry;
use crate::version::Version;
use crate::{tables, Db};

/// The database version that introduced the `BlockStateUpdates` table.
/// Databases opened with a version below this need to perform state updates migration.
///
/// The schema changes as well as the version bump were introduced in this PR: <https://github.com/dojoengine/katana/pull/470>
const STATE_UPDATES_TABLE_VERSION: Version = Version::new(9);

/// Stage ID used to checkpoint state update backfill progress in the
/// [`MigrationCheckpoints`](tables::MigrationCheckpoints) table.
const STATE_UPDATE_MIGRATION_STAGE: &str = "migration/state-updates";

pub(crate) struct StateUpdatesStage;

impl MigrationStage for StateUpdatesStage {
    fn id(&self) -> &'static str {
        STATE_UPDATE_MIGRATION_STAGE
    }

    fn threshold_version(&self) -> Version {
        STATE_UPDATES_TABLE_VERSION
    }

    fn execute(&self, db: &Db) -> Result<(), MigrationError> {
        // Determine latest block number from BlockHashes cursor
        let latest_block_number = {
            let last = db.view(|tx| tx.cursor::<tables::BlockHashes>()?.last())?;

            match last {
                Some((block_num, _)) => block_num,
                None => return Ok(()), // no blocks, nothing to migrate
            }
        };

        let total_blocks = latest_block_number + 1;

        // Resume from the last checkpoint if one exists.
        let checkpoint = db.view(|tx| {
            tx.get::<tables::MigrationCheckpoints>(STATE_UPDATE_MIGRATION_STAGE.to_string())
        })?;

        let start_block = checkpoint.map(|cp| cp.next_key).unwrap_or(0);

        if start_block >= total_blocks {
            eprintln!("[Migrating] \x1b[1;33mBlockStateUpdates\x1b[0m already up to date.");
            // Clean up the checkpoint.
            db.update(|tx| {
                tx.delete::<tables::MigrationCheckpoints>(
                    STATE_UPDATE_MIGRATION_STAGE.to_string(),
                    None,
                )?;
                Ok(())
            })?;
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
            let next_block = batch_end + 1;
            let is_last_batch = next_block > latest_block_number;

            let tx = db.tx_mut()?;
            for block in batch_start..=batch_end {
                let state_updates = reconstruct_state_update(&tx, block).map_err(|source| {
                    MigrationError::FailedToReconstructStateUpdate { block, source }
                })?;
                tx.put::<tables::BlockStateUpdates>(block, state_updates)?;
            }

            if !is_last_batch {
                tx.put::<tables::MigrationCheckpoints>(
                    STATE_UPDATE_MIGRATION_STAGE.to_string(),
                    MigrationCheckpoint { next_key: next_block },
                )?;
            } else {
                tx.delete::<tables::MigrationCheckpoints>(
                    STATE_UPDATE_MIGRATION_STAGE.to_string(),
                    None,
                )?;
            }

            tx.commit()?;

            pb.set_position(batch_end - start_block + 1);

            batch_start = next_block;
        }

        pb.finish_with_message(format!("[Migrating] Migrated {blocks_to_migrate} blocks"));

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
