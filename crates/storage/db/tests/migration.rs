use katana_db::abstraction::{Database, DbTx, DbTxMut};
use katana_db::migration::Migration;
use katana_db::models::class::MigratedCompiledClassHash;
use katana_db::models::contract::ContractClassChange;
use katana_db::models::storage::{ContractStorageEntry, ContractStorageKey};
use katana_db::version::{create_db_version_file, get_db_version, Version, LATEST_DB_VERSION};
use katana_db::{tables, Db};
use katana_primitives::state::StateUpdates;
use katana_primitives::{ContractAddress, Felt};
use katana_utils::arbitrary;

/// Creates a DB at version 8 (before BlockStateUpdates was introduced) so that
/// `Migration::is_needed()` returns true.
fn create_old_version_db() -> (Db, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    create_db_version_file(dir.path(), Version::new(8)).unwrap();
    let db = Db::open_no_sync(dir.path()).unwrap();
    (db, dir)
}

/// Write old-format index data for a single block using arbitrary values,
/// then run migration and verify all data is correctly reconstructed.
#[test]
fn migrate_reconstructs_state_updates_from_index_tables() {
    let (db, _dir) = create_old_version_db();

    let block = 0u64;

    // Generate arbitrary values for the test
    let nonce_addr: ContractAddress = arbitrary!(ContractAddress);
    let nonce_val: Felt = arbitrary!(Felt);

    let deployed1_addr: ContractAddress = arbitrary!(ContractAddress);
    let deployed1_hash: Felt = arbitrary!(Felt);
    let deployed2_addr: ContractAddress = arbitrary!(ContractAddress);
    let deployed2_hash: Felt = arbitrary!(Felt);

    // Use fixed addresses to avoid any DupSort key collision issues
    let replaced_addr: ContractAddress = ContractAddress::from(katana_primitives::felt!("0xdead"));
    let replaced_hash: Felt = arbitrary!(Felt);

    let sierra_class_hash: Felt = arbitrary!(Felt);
    let sierra_compiled_hash: Felt = arbitrary!(Felt);
    let legacy_class_hash: Felt = arbitrary!(Felt);

    let migrated_class: Felt = arbitrary!(Felt);
    let migrated_compiled: Felt = arbitrary!(Felt);

    let storage_addr: ContractAddress = arbitrary!(ContractAddress);
    let storage_key: Felt = arbitrary!(Felt);
    let storage_val: Felt = arbitrary!(Felt);

    // Write block hash
    {
        let tx = db.tx_mut().unwrap();
        tx.put::<tables::BlockHashes>(block, arbitrary!(Felt)).unwrap();
        tx.commit().unwrap();
    }

    // Write legacy index data
    {
        let tx = db.tx_mut().unwrap();

        tx.put::<tables::NonceChangeHistory>(
            block,
            katana_db::models::contract::ContractNonceChange {
                contract_address: nonce_addr,
                nonce: nonce_val,
            },
        )
        .unwrap();

        tx.put::<tables::ClassChangeHistory>(
            block,
            ContractClassChange::deployed(deployed1_addr, deployed1_hash),
        )
        .unwrap();
        tx.put::<tables::ClassChangeHistory>(
            block,
            ContractClassChange::deployed(deployed2_addr, deployed2_hash),
        )
        .unwrap();
        tx.put::<tables::ClassChangeHistory>(
            block,
            ContractClassChange::replaced(replaced_addr, replaced_hash),
        )
        .unwrap();

        // Sierra class declaration (has compiled class hash)
        tx.put::<tables::ClassDeclarations>(block, sierra_class_hash).unwrap();
        tx.put::<tables::CompiledClassHashes>(sierra_class_hash, sierra_compiled_hash).unwrap();

        // Deprecated (Cairo 0) class declaration (no compiled class hash)
        tx.put::<tables::ClassDeclarations>(block, legacy_class_hash).unwrap();

        tx.put::<tables::MigratedCompiledClassHashes>(
            block,
            MigratedCompiledClassHash {
                class_hash: migrated_class,
                compiled_class_hash: migrated_compiled,
            },
        )
        .unwrap();

        tx.put::<tables::StorageChangeHistory>(
            block,
            ContractStorageEntry {
                key: ContractStorageKey { contract_address: storage_addr, key: storage_key },
                value: storage_val,
            },
        )
        .unwrap();

        tx.commit().unwrap();
    }

    assert!(Migration::new(&db).is_needed());
    Migration::new(&db).run().unwrap();

    // Read back and verify
    let tx = db.tx().unwrap();
    let su = tx.get::<tables::BlockStateUpdates>(block).unwrap().unwrap();
    tx.commit().unwrap();

    assert_eq!(su.nonce_updates.get(&nonce_addr), Some(&nonce_val));
    assert_eq!(su.deployed_contracts.get(&deployed1_addr), Some(&deployed1_hash));
    assert_eq!(su.deployed_contracts.get(&deployed2_addr), Some(&deployed2_hash));
    assert_eq!(su.replaced_classes.get(&replaced_addr), Some(&replaced_hash));
    assert_eq!(su.declared_classes.get(&sierra_class_hash), Some(&sierra_compiled_hash));
    assert!(su.deprecated_declared_classes.contains(&legacy_class_hash));
    assert_eq!(su.migrated_compiled_classes.get(&migrated_class), Some(&migrated_compiled));
    let storage = su.storage_updates.get(&storage_addr).unwrap();
    assert_eq!(storage.get(&storage_key), Some(&storage_val));
}

#[test]
fn migration_not_needed_for_new_db() {
    let db = Db::in_memory().unwrap();
    assert!(!Migration::new(&db).is_needed());
}

#[test]
fn migration_needed_for_old_version_db() {
    let (db, _dir) = create_old_version_db();
    assert!(Migration::new(&db).is_needed());
}

#[test]
fn migration_updates_version_file() {
    let (db, dir) = create_old_version_db();

    {
        let tx = db.tx_mut().unwrap();
        tx.put::<tables::BlockHashes>(0u64, arbitrary!(Felt)).unwrap();
        tx.commit().unwrap();
    }

    assert!(Migration::new(&db).is_needed());
    Migration::new(&db).run().unwrap();

    let version = get_db_version(dir.path()).unwrap();
    assert_eq!(version, LATEST_DB_VERSION);
}

#[test]
fn migration_resumes_from_last_committed_batch() {
    let (db, _dir) = create_old_version_db();

    {
        let tx = db.tx_mut().unwrap();
        for i in 0..3u64 {
            tx.put::<tables::BlockHashes>(i, arbitrary!(Felt)).unwrap();
        }
        tx.commit().unwrap();
    }

    // Simulate partial migration: only block 0 was migrated
    {
        let tx = db.tx_mut().unwrap();
        tx.put::<tables::BlockStateUpdates>(0, StateUpdates::default()).unwrap();
        tx.commit().unwrap();
    }

    Migration::new(&db).run().unwrap();

    let tx = db.tx().unwrap();
    let count = tx.entries::<tables::BlockStateUpdates>().unwrap();
    tx.commit().unwrap();
    assert_eq!(count, 3);
}

#[test]
fn migration_handles_multiple_blocks_with_varied_data() {
    let (db, _dir) = create_old_version_db();

    let num_blocks = 5u64;

    {
        let tx = db.tx_mut().unwrap();
        for i in 0..num_blocks {
            tx.put::<tables::BlockHashes>(i, arbitrary!(Felt)).unwrap();
        }
        tx.commit().unwrap();
    }

    // Populate each block with random legacy data
    {
        let tx = db.tx_mut().unwrap();
        for block in 0..num_blocks {
            tx.put::<tables::NonceChangeHistory>(
                block,
                katana_db::models::contract::ContractNonceChange {
                    contract_address: arbitrary!(ContractAddress),
                    nonce: arbitrary!(Felt),
                },
            )
            .unwrap();

            tx.put::<tables::ClassChangeHistory>(
                block,
                ContractClassChange::deployed(arbitrary!(ContractAddress), arbitrary!(Felt)),
            )
            .unwrap();

            tx.put::<tables::StorageChangeHistory>(
                block,
                ContractStorageEntry {
                    key: ContractStorageKey {
                        contract_address: arbitrary!(ContractAddress),
                        key: arbitrary!(Felt),
                    },
                    value: arbitrary!(Felt),
                },
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }

    Migration::new(&db).run().unwrap();

    let tx = db.tx().unwrap();
    let count = tx.entries::<tables::BlockStateUpdates>().unwrap();
    tx.commit().unwrap();
    assert_eq!(count, num_blocks as usize);

    // Verify each block has non-empty state updates
    let tx = db.tx().unwrap();
    for block in 0..num_blocks {
        let su = tx.get::<tables::BlockStateUpdates>(block).unwrap().unwrap();
        assert!(!su.nonce_updates.is_empty(), "block {block} missing nonce updates");
        assert!(!su.deployed_contracts.is_empty(), "block {block} missing deployed contracts");
        assert!(!su.storage_updates.is_empty(), "block {block} missing storage updates");
    }
    tx.commit().unwrap();
}

#[test]
fn migration_empty_db_is_noop() {
    let (db, _dir) = create_old_version_db();

    Migration::new(&db).run().unwrap();

    let tx = db.tx().unwrap();
    let count = tx.entries::<tables::BlockStateUpdates>().unwrap();
    tx.commit().unwrap();
    assert_eq!(count, 0);
}

#[test]
fn migration_block_with_no_state_changes() {
    let (db, _dir) = create_old_version_db();

    {
        let tx = db.tx_mut().unwrap();
        tx.put::<tables::BlockHashes>(0u64, arbitrary!(Felt)).unwrap();
        tx.commit().unwrap();
    }

    Migration::new(&db).run().unwrap();

    let tx = db.tx().unwrap();
    let su = tx.get::<tables::BlockStateUpdates>(0u64).unwrap().unwrap();
    tx.commit().unwrap();

    assert!(su.nonce_updates.is_empty());
    assert!(su.deployed_contracts.is_empty());
    assert!(su.replaced_classes.is_empty());
    assert!(su.declared_classes.is_empty());
    assert!(su.deprecated_declared_classes.is_empty());
    assert!(su.storage_updates.is_empty());
}

#[test]
fn migration_not_needed_after_successful_run() {
    let (db, dir) = create_old_version_db();

    {
        let tx = db.tx_mut().unwrap();
        tx.put::<tables::BlockHashes>(0u64, arbitrary!(Felt)).unwrap();
        tx.commit().unwrap();
    }

    assert!(Migration::new(&db).is_needed());
    Migration::new(&db).run().unwrap();

    // Drop the first DB handle before reopening
    drop(db);

    // Reopen the DB — should no longer need migration
    let db2 = Db::open_no_sync(dir.path()).unwrap();
    assert!(!Migration::new(&db2).is_needed());
}
