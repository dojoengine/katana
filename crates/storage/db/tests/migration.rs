//! Integration tests for individual migration stages.
//!
//! These tests verify that each stage correctly transforms data — the pipeline's
//! batching, checkpointing, and version management are tested separately via unit
//! tests in `migration/mod.rs`.

use katana_db::abstraction::{Database, DbTx, DbTxMut};
use katana_db::codecs::Compress;
use katana_db::migration::Migration;
use katana_db::models::class::MigratedCompiledClassHash;
use katana_db::models::contract::ContractClassChange;
use katana_db::models::storage::{ContractStorageEntry, ContractStorageKey};
use katana_db::version::{create_db_version_file, Version};
use katana_db::{tables, Db};
use katana_primitives::receipt::{InvokeTxReceipt, Receipt};
use katana_primitives::state::StateUpdates;
use katana_primitives::transaction::TxNumber;
use katana_primitives::{ContractAddress, Felt};
use katana_utils::arbitrary;

/// Shadow table that maps to the physical `Receipts` table but uses the legacy
/// raw-postcard `Receipt` codec as the value type (instead of `ReceiptEnvelope`).
#[derive(Debug)]
struct LegacyReceipts;

impl tables::Table for LegacyReceipts {
    const NAME: &'static str = tables::Receipts::NAME;
    type Key = TxNumber;
    type Value = Receipt;
}

/// Creates a DB at version 8 (before BlockStateUpdates / ReceiptEnvelope were
/// introduced) so that all v9 migration stages are applicable.
fn create_old_version_db() -> (Db, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    create_db_version_file(dir.path(), Version::new(8)).unwrap();
    let db = Db::open_no_sync(dir.path()).unwrap();
    (db, dir)
}

fn sample_receipt(id: u64) -> Receipt {
    Receipt::Invoke(InvokeTxReceipt {
        revert_error: if id % 2 == 0 { None } else { Some(format!("revert-{id}")) },
        events: Vec::new(),
        fee: Default::default(),
        messages_sent: Vec::new(),
        execution_resources: Default::default(),
    })
}

// ---------------------------------------------------------------------------
// State updates stage
// ---------------------------------------------------------------------------

/// Write old-format index data for a single block using arbitrary values,
/// then run migration and verify all fields are correctly reconstructed.
#[test]
fn state_updates_reconstructs_all_fields() {
    let (db, _dir) = create_old_version_db();

    let block = 0u64;

    let nonce_addr: ContractAddress = arbitrary!(ContractAddress);
    let nonce_val: Felt = arbitrary!(Felt);

    let deployed1_addr: ContractAddress = arbitrary!(ContractAddress);
    let deployed1_hash: Felt = arbitrary!(Felt);
    let deployed2_addr: ContractAddress = arbitrary!(ContractAddress);
    let deployed2_hash: Felt = arbitrary!(Felt);

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

    {
        let tx = db.tx_mut().unwrap();
        tx.put::<tables::BlockHashes>(block, arbitrary!(Felt)).unwrap();
        tx.commit().unwrap();
    }

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

        tx.put::<tables::ClassDeclarations>(block, sierra_class_hash).unwrap();
        tx.put::<tables::CompiledClassHashes>(sierra_class_hash, sierra_compiled_hash).unwrap();

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

    Migration::new_v9(&db).run().unwrap();

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
fn state_updates_multiple_blocks() {
    let (db, _dir) = create_old_version_db();

    let num_blocks = 5u64;

    {
        let tx = db.tx_mut().unwrap();
        for i in 0..num_blocks {
            tx.put::<tables::BlockHashes>(i, arbitrary!(Felt)).unwrap();
        }
        tx.commit().unwrap();
    }

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

    Migration::new_v9(&db).run().unwrap();

    let tx = db.tx().unwrap();
    let count = tx.entries::<tables::BlockStateUpdates>().unwrap();
    assert_eq!(count, num_blocks as usize);

    for block in 0..num_blocks {
        let su = tx.get::<tables::BlockStateUpdates>(block).unwrap().unwrap();
        assert!(!su.nonce_updates.is_empty(), "block {block} missing nonce updates");
        assert!(!su.deployed_contracts.is_empty(), "block {block} missing deployed contracts");
        assert!(!su.storage_updates.is_empty(), "block {block} missing storage updates");
    }
    tx.commit().unwrap();
}

#[test]
fn state_updates_empty_block() {
    let (db, _dir) = create_old_version_db();

    {
        let tx = db.tx_mut().unwrap();
        tx.put::<tables::BlockHashes>(0u64, arbitrary!(Felt)).unwrap();
        tx.commit().unwrap();
    }

    Migration::new_v9(&db).run().unwrap();

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

// ---------------------------------------------------------------------------
// Receipt envelope stage
// ---------------------------------------------------------------------------

/// Write receipts using the legacy raw-postcard codec, run migration, then verify
/// they can be read back through the new `ReceiptEnvelope` codec.
#[test]
fn receipt_envelope_converts_legacy() {
    let (db, _dir) = create_old_version_db();

    // Need at least one block hash so the state-update stage doesn't skip.
    {
        let tx = db.tx_mut().unwrap();
        tx.put::<tables::BlockHashes>(0u64, arbitrary!(Felt)).unwrap();
        tx.commit().unwrap();
    }

    let receipts: Vec<Receipt> = (0..5).map(sample_receipt).collect();

    {
        let tx = db.tx_mut().unwrap();
        for (i, receipt) in receipts.iter().enumerate() {
            tx.put::<LegacyReceipts>(i as u64, receipt.clone()).unwrap();
        }
        tx.commit().unwrap();
    }

    // Sanity: reading through the envelope codec should fail before migration.
    {
        let tx = db.tx().unwrap();
        let result = tx.get::<tables::Receipts>(0u64);
        assert!(result.is_err(), "envelope codec should reject legacy postcard bytes");
        tx.commit().unwrap();
    }

    Migration::new_v9(&db).run().unwrap();

    {
        let tx = db.tx().unwrap();
        for (i, expected) in receipts.iter().enumerate() {
            let envelope = tx
                .get::<tables::Receipts>(i as u64)
                .expect("read should succeed")
                .expect("entry should exist");
            assert_eq!(&envelope.inner, expected, "receipt {i} mismatch");
        }
        tx.commit().unwrap();
    }
}

#[test]
fn receipt_envelope_empty_table() {
    let (db, _dir) = create_old_version_db();

    Migration::new_v9(&db).run().unwrap();

    let tx = db.tx().unwrap();
    let count = tx.entries::<tables::Receipts>().unwrap();
    tx.commit().unwrap();
    assert_eq!(count, 0);
}

/// Verify that migrated receipts are stored in the envelope wire format
/// (first 4 bytes == KRCP magic).
#[test]
fn receipt_envelope_wire_format() {
    let (db, _dir) = create_old_version_db();

    {
        let tx = db.tx_mut().unwrap();
        tx.put::<tables::BlockHashes>(0u64, arbitrary!(Felt)).unwrap();
        tx.put::<LegacyReceipts>(0u64, sample_receipt(0)).unwrap();
        tx.commit().unwrap();
    }

    Migration::new_v9(&db).run().unwrap();

    let tx = db.tx().unwrap();
    let envelope = tx.get::<tables::Receipts>(0u64).unwrap().unwrap();
    tx.commit().unwrap();

    let bytes = envelope.compress().expect("compress");
    assert_eq!(&bytes[..4], b"KRCP", "migrated receipt should have KRCP magic prefix");
}

// ---------------------------------------------------------------------------
// Receipt envelope migration tests
// ---------------------------------------------------------------------------

fn sample_receipt(id: u64) -> Receipt {
    Receipt::Invoke(InvokeTxReceipt {
        revert_error: if id % 2 == 0 { None } else { Some(format!("revert-{id}")) },
        events: Vec::new(),
        fee: Default::default(),
        messages_sent: Vec::new(),
        execution_resources: Default::default(),
    })
}

/// Write receipts using the legacy raw-postcard codec, run migration, then verify
/// they can be read back through the new `ReceiptEnvelope` codec.
#[test]
fn receipt_migration_converts_legacy_to_envelope() {
    let (db, _dir) = create_old_version_db();

    // Need at least one block hash so the state-update backfill doesn't skip entirely.
    {
        let tx = db.tx_mut().unwrap();
        tx.put::<tables::BlockHashes>(0u64, arbitrary!(Felt)).unwrap();
        tx.commit().unwrap();
    }

    let receipts: Vec<Receipt> = (0..5).map(sample_receipt).collect();

    // Write receipts using the legacy codec (raw postcard).
    {
        let tx = db.tx_mut().unwrap();
        for (i, receipt) in receipts.iter().enumerate() {
            tx.put::<LegacyReceipts>(i as u64, receipt.clone()).unwrap();
        }
        tx.commit().unwrap();
    }

    // Sanity: reading through the envelope codec should fail before migration
    // because the raw postcard bytes don't start with the KRCP magic.
    {
        let tx = db.tx().unwrap();
        let result = tx.get::<tables::Receipts>(0u64);
        assert!(result.is_err(), "envelope codec should reject legacy postcard bytes");
        tx.commit().unwrap();
    }

    Migration::new(&db).run().unwrap();

    // After migration, every receipt should be readable through the envelope codec.
    {
        let tx = db.tx().unwrap();
        for (i, expected) in receipts.iter().enumerate() {
            let envelope = tx
                .get::<tables::Receipts>(i as u64)
                .expect("read should succeed")
                .expect("entry should exist");
            assert_eq!(&envelope.inner, expected, "receipt {i} mismatch");
        }
        tx.commit().unwrap();
    }
}

/// An empty Receipts table should not cause the migration to fail.
#[test]
fn receipt_migration_empty_table_is_noop() {
    let (db, _dir) = create_old_version_db();

    Migration::new(&db).run().unwrap();

    let tx = db.tx().unwrap();
    let count = tx.entries::<tables::Receipts>().unwrap();
    tx.commit().unwrap();
    assert_eq!(count, 0);
}

/// Verify that migrated receipts are stored in the envelope wire format
/// (first 4 bytes == KRCP magic).
#[test]
fn receipt_migration_produces_envelope_wire_format() {
    let (db, _dir) = create_old_version_db();

    {
        let tx = db.tx_mut().unwrap();
        tx.put::<tables::BlockHashes>(0u64, arbitrary!(Felt)).unwrap();
        tx.put::<LegacyReceipts>(0u64, sample_receipt(0)).unwrap();
        tx.commit().unwrap();
    }

    Migration::new(&db).run().unwrap();

    // Read the migrated value back through the envelope codec and re-compress
    // to inspect the wire format.
    let tx = db.tx().unwrap();
    let envelope = tx.get::<tables::Receipts>(0u64).unwrap().unwrap();
    tx.commit().unwrap();

    let bytes = envelope.compress().expect("compress");
    assert_eq!(&bytes[..4], b"KRCP", "migrated receipt should have KRCP magic prefix");
}

/// Migration with more receipts than a single batch (BATCH_SIZE = 1000) to exercise
/// the batching loop.
#[test]
fn receipt_migration_handles_multiple_batches() {
    let (db, _dir) = create_old_version_db();

    let num_receipts: u64 = 1500;

    {
        let tx = db.tx_mut().unwrap();
        tx.put::<tables::BlockHashes>(0u64, arbitrary!(Felt)).unwrap();
        for i in 0..num_receipts {
            tx.put::<LegacyReceipts>(i, sample_receipt(i)).unwrap();
        }
        tx.commit().unwrap();
    }

    Migration::new(&db).run().unwrap();

    let tx = db.tx().unwrap();
    let count = tx.entries::<tables::Receipts>().unwrap();
    tx.commit().unwrap();
    assert_eq!(count, num_receipts as usize);

    // Spot check first, last, and a middle entry.
    let tx = db.tx().unwrap();
    for i in [0, 749, num_receipts - 1] {
        let envelope = tx.get::<tables::Receipts>(i).unwrap().unwrap();
        assert_eq!(envelope.inner, sample_receipt(i), "receipt {i} mismatch");
    }
    tx.commit().unwrap();
}

/// Migration should not be needed for a database already at the current version.
#[test]
fn receipt_migration_not_needed_at_current_version() {
    let db = Db::in_memory().unwrap();
    assert!(!Migration::new(&db).is_needed());
}
