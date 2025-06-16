use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use katana::cli::Cli;
use katana_db::abstraction::{Database, DbCursor, DbTx, DbTxMut};
use katana_db::mdbx::DbEnv;
use katana_db::tables::{self};
use katana_primitives::block::Header;
use katana_primitives::class::{ClassHash, CompiledClassHash};
use katana_primitives::contract::{ContractAddress, Nonce, StorageKey, StorageValue};
use katana_primitives::state::StateUpdates;
use katana_primitives::Felt;
use katana_provider::providers::db::DbProvider;
use katana_provider::traits::state::{StateFactoryProvider, StateRootProvider};
use katana_provider::traits::trie::TrieWriter;
use rstest::*;
use tempfile::TempDir;

struct TempDb {
    #[allow(unused)]
    temp_dir: TempDir,
    db_path: PathBuf,
}

impl TempDb {
    fn new() -> Self {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let db_path = temp_dir.path().join("test_db");
        katana_db::init_db(&db_path).expect("failed to initialize database");
        Self { temp_dir, db_path }
    }

    fn provider_ro(&self) -> DbProvider {
        DbProvider::new(self.open_ro())
    }

    fn provider_rw(&self) -> DbProvider {
        DbProvider::new(self.open_rw())
    }

    fn open_ro(&self) -> DbEnv {
        katana::cli::db::open_db_ro(self.db_path.to_str().unwrap()).unwrap()
    }

    fn open_rw(&self) -> DbEnv {
        katana::cli::db::open_db_rw(self.db_path.to_str().unwrap()).unwrap()
    }

    fn path_str(&self) -> &str {
        self.db_path.to_str().unwrap()
    }
}

/// Helper to create an empty temporary database
#[fixture]
fn empty_db() -> TempDb {
    TempDb::new()
}

/// Helper to create a temporary database with arbitrary generated data
#[fixture]
fn db() -> TempDb {
    let db = TempDb::new();
    // Initialize with arbitrary test data
    populate_db(&db).expect("failed to populate test database");
    db
}

/// Generate test addresses and class hashes
fn generate_test_addresses(count: usize) -> Vec<ContractAddress> {
    (0..count).map(|i| ContractAddress::from(Felt::from(i))).collect()
}

fn generate_test_class_hashes(count: usize) -> Vec<ClassHash> {
    (0..count).map(|i| ClassHash::from(i as u128 + 1000)).collect()
}

fn generate_test_compiled_class_hashes(count: usize) -> Vec<CompiledClassHash> {
    (0..count).map(|i| CompiledClassHash::from(i as u128 + 2000)).collect()
}

/// Populate database with test data using the TrieWriter trait
fn populate_db(db: &TempDb) -> Result<()> {
    let provider = db.provider_rw();

    // Generate test data
    let num_contracts = 5;
    let num_classes = 3;
    let addresses = generate_test_addresses(num_contracts);
    let class_hashes = generate_test_class_hashes(num_classes);
    let compiled_class_hashes = generate_test_compiled_class_hashes(num_classes);

    // Insert headers first
    db.open_rw()
        .update(|tx| {
            for block_num in 0..=15u64 {
                tx.put::<tables::Headers>(block_num, Header::default()).unwrap();
            }
        })
        .expect("failed to insert headers");

    // Process each block to create historical trie data
    for block_number in 1..=15u64 {
        let mut state_updates = StateUpdates::default();

        // Add some declared classes for this block (only for some blocks)
        if block_number % 3 == 0 && (block_number / 3) as usize <= num_classes {
            let idx = ((block_number / 3) - 1) as usize;
            state_updates.declared_classes.insert(class_hashes[idx], compiled_class_hashes[idx]);
        }

        // Add some deployed contracts for this block (only for some blocks)
        if block_number % 2 == 0 && (block_number / 2) as usize <= num_contracts {
            let idx = ((block_number / 2) - 1) as usize;
            let class_idx = idx % num_classes;
            state_updates.deployed_contracts.insert(addresses[idx], class_hashes[class_idx]);
            // Also set initial nonce
            state_updates.nonce_updates.insert(addresses[idx], Nonce::from(1u8));
        }

        // Add storage updates for existing contracts
        for (idx, &address) in addresses.iter().enumerate() {
            if idx < (block_number as usize).min(num_contracts) {
                let mut storage_entries = BTreeMap::new();
                // Add a few storage entries that change each block
                for storage_idx in 0..3 {
                    let key = StorageKey::from(storage_idx as u128);
                    let value = StorageValue::from((block_number * 100 + storage_idx) as u128);
                    storage_entries.insert(key, value);
                }
                state_updates.storage_updates.insert(address, storage_entries);

                // Update nonce
                state_updates.nonce_updates.insert(address, Nonce::from(block_number as u8));
            }
        }

        // Insert declared classes into the trie
        if !state_updates.declared_classes.is_empty() {
            provider
                .trie_insert_declared_classes(block_number, &state_updates.declared_classes)
                .context("failed to insert declared classes")?;
        }

        // Insert contract updates into the trie
        provider
            .trie_insert_contract_updates(block_number, &state_updates)
            .context("failed to insert contract updates")?;
    }

    Ok(())
}

/// Count total historical entries in the database
fn count_historical_entries(db: &DbEnv) -> Result<usize> {
    let tx = db.tx()?;
    let mut total = 0;

    // Count ClassesTrieHistory entries
    let mut cursor = tx.cursor_dup::<tables::ClassesTrieHistory>()?;
    let mut walker = cursor.walk(None)?;
    while walker.next().transpose()?.is_some() {
        total += 1;
    }

    // Count ContractsTrieHistory entries
    let mut cursor = tx.cursor_dup::<tables::ContractsTrieHistory>()?;
    let mut walker = cursor.walk(None)?;
    while walker.next().transpose()?.is_some() {
        total += 1;
    }

    // Count StoragesTrieHistory entries
    let mut cursor = tx.cursor_dup::<tables::StoragesTrieHistory>()?;
    let mut walker = cursor.walk(None)?;
    while walker.next().transpose()?.is_some() {
        total += 1;
    }

    Ok(total)
}

/// Count headers in the database (should remain unchanged after pruning)
fn count_headers(db: &DbEnv) -> Result<usize> {
    let tx = db.tx()?;
    let mut cursor = tx.cursor::<tables::Headers>()?;
    let mut count = 0;

    let mut walker = cursor.walk(None)?;
    while walker.next().transpose()?.is_some() {
        count += 1;
    }

    Ok(count)
}

/// Get the current state roots (classes and contracts)
fn get_state_roots<Db: Database>(provider: &DbProvider<Db>) -> Result<(Felt, Felt)> {
    let state_provider = provider.latest().context("failed to get latest state provider")?;
    let classes_root = state_provider.classes_root().context("failed to get classes root")?;
    let contracts_root = state_provider.contracts_root().context("failed to get contracts root")?;
    Ok((classes_root, contracts_root))
}

#[rstest]
fn prune_latest_removes_all_history(db: TempDb) {
    ///////////////////////////////////////////////////////////////////////////////////
    // Verify we have historical data before pruning and capture state roots
    ///////////////////////////////////////////////////////////////////////////////////
    let provider = db.provider_ro();
    let (initial_classes_root, initial_contracts_root) = get_state_roots(&provider).unwrap();
    drop(provider);

    let env = db.open_ro();
    let initial_history_count = count_historical_entries(&env).unwrap();
    let initial_headers_count = count_headers(&env).unwrap();

    assert!(initial_history_count > 0, "Should have historical data before pruning");
    assert_eq!(initial_headers_count, 16);
    drop(env);
    ///////////////////////////////////////////////////////////////////////////////////

    let path = db.path_str();
    Cli::parse_from(["katana", "db", "prune", "--path", path, "latest"]).run().unwrap();

    ///////////////////////////////////////////////////////////////////////////////////
    // Verify pruning outcome and state roots remain unchanged
    ///////////////////////////////////////////////////////////////////////////////////
    let provider = db.provider_ro();
    let (final_classes_root, final_contracts_root) = get_state_roots(&provider).unwrap();
    drop(provider);

    let env = db.open_ro();
    let final_history_count = count_historical_entries(&env).unwrap();
    let final_headers_count = count_headers(&env).unwrap();

    assert_eq!(final_history_count, 0, "All historical data should be removed");
    assert_eq!(final_headers_count, initial_headers_count, "Headers should be unchanged");
    assert_eq!(
        final_classes_root, initial_classes_root,
        "Classes root should remain unchanged after pruning"
    );
    assert_eq!(
        final_contracts_root, initial_contracts_root,
        "Contracts root should remain unchanged after pruning"
    );
    drop(env);
    ///////////////////////////////////////////////////////////////////////////////////
}

#[rstest]
fn prune_keep_last_n_blocks(db: TempDb) {
    ///////////////////////////////////////////////////////////////////////////////////
    // Verify we have historical data before pruning and capture state roots
    ///////////////////////////////////////////////////////////////////////////////////
    let provider = db.provider_ro();
    let (initial_classes_root, initial_contracts_root) = get_state_roots(&provider).unwrap();
    drop(provider);

    let env = db.open_ro();
    let initial_history_count = count_historical_entries(&env).unwrap();
    let initial_headers_count = count_headers(&env).unwrap();

    assert!(initial_history_count > 0, "Should have historical data before pruning");
    assert_eq!(initial_headers_count, 16);
    drop(env);
    ///////////////////////////////////////////////////////////////////////////////////

    let path = db.path_str();
    Cli::parse_from(["katana", "db", "prune", "--path", path, "keep-last-n", "3"]).run().unwrap();

    ///////////////////////////////////////////////////////////////////////////////////
    // Verify pruning outcome and state roots remain unchanged
    ///////////////////////////////////////////////////////////////////////////////////
    let provider = db.provider_ro();
    let (final_classes_root, final_contracts_root) = get_state_roots(&provider).unwrap();
    drop(provider);

    let env = db.open_ro();
    let final_history_count = count_historical_entries(&env).unwrap();
    let final_headers_count = count_headers(&env).unwrap();

    assert!(
        final_history_count < initial_history_count,
        "Historical data should be reduced after pruning"
    );
    assert!(final_history_count > 0, "Some historical data should remain (last 3 blocks)");
    assert_eq!(final_headers_count, initial_headers_count, "Headers should be unchanged");
    assert_eq!(
        final_classes_root, initial_classes_root,
        "Classes root should remain unchanged after pruning"
    );
    assert_eq!(
        final_contracts_root, initial_contracts_root,
        "Contracts root should remain unchanged after pruning"
    );
    drop(env);
    ///////////////////////////////////////////////////////////////////////////////////
}
