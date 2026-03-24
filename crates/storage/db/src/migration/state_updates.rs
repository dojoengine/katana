use std::collections::BTreeMap;
use std::ops::RangeInclusive;

use katana_primitives::block::BlockNumber;
use katana_primitives::contract::{StorageKey, StorageValue};
use katana_primitives::state::StateUpdates;

use super::{MigrationError, MigrationStage};
use crate::abstraction::{Database, DbCursor, DbDupSortCursor, DbTx, DbTxMut};
use crate::error::DatabaseError;
use crate::mdbx::tx::TxRW;
use crate::models::class::MigratedCompiledClassHash;
use crate::models::contract::{ContractClassChange, ContractClassChangeType};
use crate::models::state_update::StateUpdateEnvelope;
use crate::models::storage::ContractStorageEntry;
use crate::version::Version;
use crate::{tables, Db};

/// The database version that introduced the `BlockStateUpdates` table.
/// Databases opened with a version below this need to perform state updates migration.
///
/// The schema changes as well as the version bump were introduced in this PR: <https://github.com/dojoengine/katana/pull/470>
const STATE_UPDATES_TABLE_VERSION: Version = Version::new(9);

pub(crate) struct StateUpdatesStage;

impl MigrationStage for StateUpdatesStage {
    fn id(&self) -> &'static str {
        "migration/state-updates"
    }

    fn threshold_version(&self) -> Version {
        STATE_UPDATES_TABLE_VERSION
    }

    fn range(&self, db: &Db) -> Result<Option<RangeInclusive<u64>>, MigrationError> {
        let last = db.view(|tx| tx.cursor::<tables::BlockHashes>()?.last())?;
        match last {
            Some((block_num, _)) => Ok(Some(0..=block_num)),
            None => Ok(None),
        }
    }

    fn execute(&self, tx: &TxRW, range: RangeInclusive<u64>) -> Result<(), MigrationError> {
        for block in range {
            let state_updates = reconstruct_state_update(tx, block).map_err(|source| {
                MigrationError::FailedToReconstructStateUpdate { block, source }
            })?;
            tx.put::<tables::BlockStateUpdates>(block, StateUpdateEnvelope::from(state_updates))?;
        }
        Ok(())
    }
}

/// Collects all DupSort entries for a given primary key from a DupSort table.
fn dup_entries<T: tables::DupSort>(tx: &TxRW, key: T::Key) -> Result<Vec<T::Value>, DatabaseError> {
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
fn reconstruct_state_update(
    tx: &TxRW,
    block_number: BlockNumber,
) -> Result<StateUpdates, DatabaseError> {
    let mut state_updates = StateUpdates::default();

    // --- Nonce updates ---
    for nonce_change in dup_entries::<tables::NonceChangeHistory>(tx, block_number)? {
        state_updates.nonce_updates.insert(nonce_change.contract_address, nonce_change.nonce);
    }

    // --- Class changes (deployed contracts + replaced classes) ---
    for class_change in dup_entries::<tables::ClassChangeHistory>(tx, block_number)? {
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
    for class_hash in dup_entries::<tables::ClassDeclarations>(tx, block_number)? {
        if let Some(compiled_class_hash) = tx.get::<tables::CompiledClassHashes>(class_hash)? {
            state_updates.declared_classes.insert(class_hash, compiled_class_hash);
        } else {
            state_updates.deprecated_declared_classes.insert(class_hash);
        }
    }

    // --- Migrated compiled class hashes ---
    for migrated in dup_entries::<tables::MigratedCompiledClassHashes>(tx, block_number)? {
        let MigratedCompiledClassHash { class_hash, compiled_class_hash } = migrated;
        state_updates.migrated_compiled_classes.insert(class_hash, compiled_class_hash);
    }

    // --- Storage updates ---
    {
        let mut storage_map: BTreeMap<_, BTreeMap<StorageKey, StorageValue>> = BTreeMap::new();
        for entry in dup_entries::<tables::StorageChangeHistory>(tx, block_number)? {
            let ContractStorageEntry { key, value } = entry;
            storage_map.entry(key.contract_address).or_default().insert(key.key, value);
        }
        state_updates.storage_updates = storage_map;
    }

    Ok(state_updates)
}
