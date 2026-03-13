use std::collections::BTreeMap;

use anyhow::Context;
use katana_primitives::block::BlockNumber;
use katana_primitives::state::StateUpdates;

use crate::abstraction::{Database, DbCursor, DbDupSortCursor, DbTx, DbTxMut};
use crate::models::contract::ContractClassChangeType;
use crate::{tables, Db};

const BATCH_SIZE: u64 = 1000;

/// Returns `true` if the database needs the `BlockStateUpdates` table to be backfilled.
///
/// Detection: `BlockStateUpdates` is empty while `BlockHashes` has entries.
pub fn needs_state_update_migration(db: &Db) -> anyhow::Result<bool> {
    let tx = db.tx().context("opening read tx for migration check")?;
    let state_update_count = tx.entries::<tables::BlockStateUpdates>()?;
    let block_count = tx.entries::<tables::BlockHashes>()?;
    tx.commit()?;
    Ok(state_update_count == 0 && block_count > 0)
}

/// Backfills the `BlockStateUpdates` table by reconstructing state updates from the legacy
/// index tables (`NonceChangeHistory`, `ClassChangeHistory`, `ClassDeclarations`,
/// `CompiledClassHashes`, `MigratedCompiledClassHashes`, `StorageChangeHistory`).
///
/// Processes blocks in batches for crash recovery — on restart, already-committed batches
/// are skipped automatically.
pub fn migrate_state_updates(db: &Db) -> anyhow::Result<()> {
    // Determine latest block number from BlockHashes cursor
    let latest_block_number = {
        let tx = db.tx().context("opening read tx for latest block")?;
        let mut cursor = tx.cursor::<tables::BlockHashes>()?;
        let last = cursor.last()?;
        tx.commit()?;
        match last {
            Some((block_num, _)) => block_num,
            None => return Ok(()), // no blocks, nothing to migrate
        }
    };

    // Determine resume point: how many entries already exist in BlockStateUpdates
    let existing_count = {
        let tx = db.tx().context("opening read tx for resume point")?;
        let count = tx.entries::<tables::BlockStateUpdates>()? as u64;
        tx.commit()?;
        count
    };

    let start_block = existing_count;
    let total_blocks = latest_block_number + 1;

    if start_block >= total_blocks {
        tracing::info!(target: "db::migration", "BlockStateUpdates already up to date.");
        return Ok(());
    }

    tracing::info!(
        target: "db::migration",
        start_block,
        latest_block_number,
        "Migrating BlockStateUpdates table..."
    );

    // Process in batches
    let mut batch_start = start_block;
    while batch_start <= latest_block_number {
        let batch_end = std::cmp::min(batch_start + BATCH_SIZE - 1, latest_block_number);

        let tx = db.tx_mut().context("opening write tx for migration batch")?;

        for block_number in batch_start..=batch_end {
            let state_updates = reconstruct_state_update(&tx, block_number)?;
            tx.put::<tables::BlockStateUpdates>(block_number, state_updates)?;
        }

        tx.commit()?;

        tracing::info!(
            target: "db::migration",
            from = batch_start,
            to = batch_end,
            total = latest_block_number,
            "Migrated blocks."
        );

        batch_start = batch_end + 1;
    }

    tracing::info!(
        target: "db::migration",
        total_blocks = latest_block_number + 1 - start_block,
        "BlockStateUpdates migration complete."
    );

    Ok(())
}

/// Collects all DupSort entries for a given primary key from a DupSort table.
fn dup_entries<Tx, T>(tx: &Tx, key: T::Key) -> anyhow::Result<Vec<T::Value>>
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

/// Reconstructs a `StateUpdates` for a single block from the legacy index tables.
fn reconstruct_state_update<Tx: DbTx>(
    tx: &Tx,
    block_number: BlockNumber,
) -> anyhow::Result<StateUpdates> {
    let mut state_updates = StateUpdates::default();

    // --- Nonce updates ---
    for nonce_change in dup_entries::<Tx, tables::NonceChangeHistory>(tx, block_number)? {
        state_updates.nonce_updates.insert(nonce_change.contract_address, nonce_change.nonce);
    }

    // --- Class changes (deployed contracts + replaced classes) ---
    for class_change in dup_entries::<Tx, tables::ClassChangeHistory>(tx, block_number)? {
        match class_change.r#type {
            ContractClassChangeType::Deployed => {
                state_updates
                    .deployed_contracts
                    .insert(class_change.contract_address, class_change.class_hash);
            }
            ContractClassChangeType::Replaced => {
                state_updates
                    .replaced_classes
                    .insert(class_change.contract_address, class_change.class_hash);
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
        state_updates
            .migrated_compiled_classes
            .insert(migrated.class_hash, migrated.compiled_class_hash);
    }

    // --- Storage updates ---
    {
        let mut storage_map: BTreeMap<_, BTreeMap<_, _>> = BTreeMap::new();
        for entry in dup_entries::<Tx, tables::StorageChangeHistory>(tx, block_number)? {
            storage_map
                .entry(entry.key.contract_address)
                .or_default()
                .insert(entry.key.key, entry.value);
        }
        state_updates.storage_updates = storage_map;
    }

    Ok(state_updates)
}

#[cfg(test)]
mod tests {
    use katana_primitives::class::{ClassHash, CompiledClassHash};
    use katana_primitives::{address, felt};

    use super::*;
    use crate::models::class::MigratedCompiledClassHash;
    use crate::models::contract::{ContractClassChange, ContractNonceChange};
    use crate::models::storage::{ContractStorageEntry, ContractStorageKey};

    /// Helper: write old-format index data for a single block, then run migration and verify.
    #[test]
    fn migrate_reconstructs_state_updates_from_index_tables() {
        let db = Db::in_memory().unwrap();

        let block: BlockNumber = 0;

        // Write a block hash so the block "exists"
        {
            let tx = db.tx_mut().unwrap();
            tx.put::<tables::BlockHashes>(block, felt!("0xdeadbeef")).unwrap();
            tx.commit().unwrap();
        }

        // Write legacy index data
        {
            let tx = db.tx_mut().unwrap();

            // Nonce update
            tx.put::<tables::NonceChangeHistory>(
                block,
                ContractNonceChange { contract_address: address!("0x1"), nonce: felt!("0x5") },
            )
            .unwrap();

            // Deployed contracts
            tx.put::<tables::ClassChangeHistory>(
                block,
                ContractClassChange::deployed(address!("0x1"), felt!("0xc1a55")),
            )
            .unwrap();

            tx.put::<tables::ClassChangeHistory>(
                block,
                ContractClassChange::deployed(address!("0x2"), felt!("0xc1a56")),
            )
            .unwrap();

            // Sierra class declaration (has compiled class hash)
            let class_hash: ClassHash = felt!("0xd1");
            let compiled_hash: CompiledClassHash = felt!("0xd1c");
            tx.put::<tables::ClassDeclarations>(block, class_hash).unwrap();
            tx.put::<tables::CompiledClassHashes>(class_hash, compiled_hash).unwrap();

            // Deprecated (Cairo 0) class declaration (no compiled class hash)
            let legacy_class: ClassHash = felt!("0xd2");
            tx.put::<tables::ClassDeclarations>(block, legacy_class).unwrap();

            // Migrated compiled class hash
            tx.put::<tables::MigratedCompiledClassHashes>(
                block,
                MigratedCompiledClassHash {
                    class_hash: felt!("0xe1"),
                    compiled_class_hash: felt!("0xe1c"),
                },
            )
            .unwrap();

            // Storage update
            tx.put::<tables::StorageChangeHistory>(
                block,
                ContractStorageEntry {
                    key: ContractStorageKey {
                        contract_address: address!("0x1"),
                        key: felt!("0x10"),
                    },
                    value: felt!("0x99"),
                },
            )
            .unwrap();

            tx.commit().unwrap();
        }

        // Verify migration is needed
        assert!(needs_state_update_migration(&db).unwrap());

        // Run migration
        migrate_state_updates(&db).unwrap();

        // Verify migration is no longer needed
        assert!(!needs_state_update_migration(&db).unwrap());

        // Read back and verify
        let tx = db.tx().unwrap();
        let state_updates = tx.get::<tables::BlockStateUpdates>(block).unwrap().unwrap();
        tx.commit().unwrap();

        // Nonce
        assert_eq!(state_updates.nonce_updates.get(&address!("0x1")), Some(&felt!("0x5")));

        // Deployed contracts
        assert_eq!(state_updates.deployed_contracts.get(&address!("0x1")), Some(&felt!("0xc1a55")));
        assert_eq!(state_updates.deployed_contracts.get(&address!("0x2")), Some(&felt!("0xc1a56")));

        // Sierra declaration
        assert_eq!(state_updates.declared_classes.get(&felt!("0xd1")), Some(&felt!("0xd1c")));

        // Deprecated declaration
        assert!(state_updates.deprecated_declared_classes.contains(&felt!("0xd2")));

        // Migrated compiled class
        assert_eq!(
            state_updates.migrated_compiled_classes.get(&felt!("0xe1")),
            Some(&felt!("0xe1c"))
        );

        // Storage
        let storage = state_updates.storage_updates.get(&address!("0x1")).unwrap();
        assert_eq!(storage.get(&felt!("0x10")), Some(&felt!("0x99")));
    }

    #[test]
    fn migration_not_needed_for_empty_db() {
        let db = Db::in_memory().unwrap();
        assert!(!needs_state_update_migration(&db).unwrap());
    }

    #[test]
    fn migration_not_needed_when_already_populated() {
        let db = Db::in_memory().unwrap();

        {
            let tx = db.tx_mut().unwrap();
            tx.put::<tables::BlockHashes>(0, felt!("0xabc")).unwrap();
            tx.put::<tables::BlockStateUpdates>(0, StateUpdates::default()).unwrap();
            tx.commit().unwrap();
        }

        assert!(!needs_state_update_migration(&db).unwrap());
    }

    #[test]
    fn migration_resumes_from_last_committed_batch() {
        let db = Db::in_memory().unwrap();

        // Create 3 blocks
        {
            let tx = db.tx_mut().unwrap();
            for i in 0..3u64 {
                tx.put::<tables::BlockHashes>(i, felt!("0xabc")).unwrap();
            }
            tx.commit().unwrap();
        }

        // Simulate partial migration: only block 0 was migrated
        {
            let tx = db.tx_mut().unwrap();
            tx.put::<tables::BlockStateUpdates>(0, StateUpdates::default()).unwrap();
            tx.commit().unwrap();
        }

        // Still needs migration (1 entry vs 3 blocks)
        assert!(needs_state_update_migration(&db).is_ok());

        // Run migration — should resume from block 1
        migrate_state_updates(&db).unwrap();

        // Verify all 3 blocks now have state updates
        let tx = db.tx().unwrap();
        let count = tx.entries::<tables::BlockStateUpdates>().unwrap();
        tx.commit().unwrap();
        assert_eq!(count, 3);
    }
}
