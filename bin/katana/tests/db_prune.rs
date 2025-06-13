use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use arbitrary::{Arbitrary, Unstructured};
use clap::Parser;
use katana::cli::db::open_db_ro;
use katana::cli::Cli;
use katana_db::abstraction::{
    Database, DbCursor, DbDupSortCursor, DbDupSortCursorMut, DbTx, DbTxMut,
};
use katana_db::mdbx::DbEnv;
use katana_db::models::trie::{TrieDatabaseKey, TrieDatabaseValue, TrieHistoryEntry};
use katana_db::tables::{ClassesTrieHistory, ContractsTrieHistory, Headers, StoragesTrieHistory};
use katana_primitives::block::{BlockHash, Header};
use rstest::*;
use tempfile::TempDir;

/// Get the latest block number from the Headers table
pub fn get_latest_block_number<Tx: DbTx>(tx: &Tx) -> Result<u64> {
    let mut cursor = tx.cursor::<Headers>()?;
    if let Some((block_num, _)) = cursor.last()? {
        Ok(block_num)
    } else {
        Ok(0)
    }
}

struct TempDb {
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

    fn open_ro(&self) -> DbEnv {
        katana::cli::db::open_db_ro(self.db_path.to_str().unwrap()).unwrap()
    }

    fn open_rw(&self) -> DbEnv {
        katana::cli::db::open_db_rw(self.db_path.to_str().unwrap()).unwrap()
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
    populate_test_database_with_arbitrary_data(&db).expect("failed to populate test database");
    db
}

/// Generate arbitrary test data for a block range
fn generate_arbitrary_trie_entries(
    num_blocks: u64,
    entries_per_block: usize,
    seed: u64,
) -> Result<Vec<(u64, Vec<TrieHistoryEntry>)>> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut blocks_data = Vec::new();

    for block_num in 1..=num_blocks {
        let mut block_entries = Vec::new();

        // Create deterministic but pseudo-random data for each block
        for entry_idx in 0..entries_per_block {
            let mut hasher = DefaultHasher::new();
            seed.hash(&mut hasher);
            block_num.hash(&mut hasher);
            entry_idx.hash(&mut hasher);
            let entry_seed = hasher.finish();

            // Create deterministic arbitrary data
            let mut data = vec![0u8; 1024]; // Enough bytes for arbitrary generation
            for (i, byte) in data.iter_mut().enumerate() {
                *byte = ((entry_seed.wrapping_add(i as u64)) % 256) as u8;
            }

            let mut unstructured = Unstructured::new(&data);
            let key = TrieDatabaseKey::arbitrary(&mut unstructured)
                .map_err(|e| anyhow::anyhow!("Failed to generate arbitrary key: {:?}", e))?;

            // Generate arbitrary value
            let value_size = 32 + (entry_seed % 64) as usize; // 32-96 bytes
            let value_data: Vec<u8> = (0..value_size)
                .map(|i| ((entry_seed.wrapping_add(i as u64 + 1000)) % 256) as u8)
                .collect();
            let value = TrieDatabaseValue::from(value_data);

            block_entries.push(TrieHistoryEntry { key, value });
        }

        blocks_data.push((block_num, block_entries));
    }

    Ok(blocks_data)
}

/// Populate database with arbitrary generated data
fn populate_test_database_with_arbitrary_data(db: &TempDb) -> Result<()> {
    let db = db.open_rw();
    let tx = db.tx_mut()?;

    // Create headers for blocks 0-15 using Arbitrary trait
    for block_num in 0..=15u64 {
        // Generate deterministic arbitrary data for headers
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(&block_num, &mut hasher);
        std::hash::Hash::hash(&"header", &mut hasher);
        let seed = std::hash::Hasher::finish(&hasher);

        let mut data = vec![0u8; 1024];
        for (i, byte) in data.iter_mut().enumerate() {
            *byte = ((seed.wrapping_add(i as u64)) % 256) as u8;
        }

        let mut unstructured = Unstructured::new(&data);
        let mut header = Header::arbitrary(&mut unstructured)
            .map_err(|e| anyhow::anyhow!("Failed to generate arbitrary header: {:?}", e))?;

        // Override specific fields to maintain blockchain invariants
        header.number = block_num;
        header.parent_hash = if block_num == 0 {
            BlockHash::ZERO
        } else {
            BlockHash::from_bytes_be(&[(block_num - 1) as u8; 32])
        };

        tx.put::<Headers>(block_num, header)?;
    }

    // Generate arbitrary trie data for each trie type
    let classes_data = generate_arbitrary_trie_entries(15, 5, 12345)?; // seed 12345
    let contracts_data = generate_arbitrary_trie_entries(15, 3, 67890)?; // seed 67890
    let storages_data = generate_arbitrary_trie_entries(15, 7, 11111)?; // seed 11111

    // Insert classes trie history
    for (block_num, entries) in classes_data {
        for entry in entries {
            tx.put::<ClassesTrieHistory>(block_num, entry)?;
        }
    }

    // Insert contracts trie history
    for (block_num, entries) in contracts_data {
        for entry in entries {
            tx.put::<ContractsTrieHistory>(block_num, entry)?;
        }
    }

    // Insert storages trie history
    for (block_num, entries) in storages_data {
        for entry in entries {
            tx.put::<StoragesTrieHistory>(block_num, entry)?;
        }
    }

    tx.commit()?;
    Ok(())
}

/// Count total historical entries in the database
fn count_historical_entries(db: &DbEnv) -> Result<usize> {
    let tx = db.tx()?;
    let mut total = 0;

    // Count ClassesTrieHistory entries
    let mut cursor = tx.cursor_dup::<ClassesTrieHistory>()?;
    let mut walker = cursor.walk(None)?;
    while walker.next().transpose()?.is_some() {
        total += 1;
    }

    // Count ContractsTrieHistory entries
    let mut cursor = tx.cursor_dup::<ContractsTrieHistory>()?;
    let mut walker = cursor.walk(None)?;
    while walker.next().transpose()?.is_some() {
        total += 1;
    }

    // Count StoragesTrieHistory entries
    let mut cursor = tx.cursor_dup::<StoragesTrieHistory>()?;
    let mut walker = cursor.walk(None)?;
    while walker.next().transpose()?.is_some() {
        total += 1;
    }

    Ok(total)
}

/// Count headers in the database (should remain unchanged after pruning)
fn count_headers(db: &DbEnv) -> Result<usize> {
    let tx = db.tx()?;
    let mut cursor = tx.cursor::<Headers>()?;
    let mut count = 0;

    let mut walker = cursor.walk(None)?;
    while walker.next().transpose()?.is_some() {
        count += 1;
    }

    Ok(count)
}

/// Count historical entries for specific blocks
fn count_historical_entries_for_blocks(
    db: &DbEnv,
    start_block: u64,
    end_block: u64,
) -> Result<usize> {
    let tx = db.tx()?;
    let mut total = 0;

    for block_num in start_block..=end_block {
        // Check ClassesTrieHistory
        let mut cursor = tx.cursor_dup::<ClassesTrieHistory>()?;
        if let Some((_, _)) = cursor.seek(block_num)? {
            // Count all entries for this block
            loop {
                total += 1;
                if cursor.next_dup()?.is_none() {
                    break;
                }
            }
        }

        // Check ContractsTrieHistory
        let mut cursor = tx.cursor_dup::<ContractsTrieHistory>()?;
        if let Some((_, _)) = cursor.seek(block_num)? {
            loop {
                total += 1;
                if cursor.next_dup()?.is_none() {
                    break;
                }
            }
        }

        // Check StoragesTrieHistory
        let mut cursor = tx.cursor_dup::<StoragesTrieHistory>()?;
        if let Some((_, _)) = cursor.seek(block_num)? {
            loop {
                total += 1;
                if cursor.next_dup()?.is_none() {
                    break;
                }
            }
        }
    }

    Ok(total)
}

#[rstest]
fn cli_prune_latest_removes_all_history(
    temp_db_with_data: (TempDir, PathBuf, Arc<DbEnv>),
) -> Result<()> {
    let (_temp_dir, db_path, db) = temp_db_with_data;

    // Verify we have historical data before pruning
    let initial_history_count = count_historical_entries(&db)?;
    let initial_headers_count = count_headers(&db)?;
    assert!(initial_history_count > 0);
    assert_eq!(initial_headers_count, 11); // blocks 0-10
                                           // Close the database before CLI command
    drop(db);

    // Parse CLI command and execute
    let cli =
        Cli::parse_from(["katana", "db", "--path", db_path.to_str().unwrap(), "prune", "latest"])
            .run()
            .expect("failed to run");

    // Reopen database to verify result
    let db_reopened = open_db_ro(&db_path.to_str().unwrap())?;
    assert_eq!(count_historical_entries(&db_reopened)?, 0);
    assert_eq!(count_headers(&db_reopened)?, initial_headers_count); // Headers should be unchanged

    Ok(())
}

#[rstest]
fn cli_prune_keep_last_n_blocks(temp_db_with_data: (TempDir, PathBuf, Arc<DbEnv>)) {
    let (_temp_dir, db_path, db) = temp_db_with_data;
    let initial_history_count = count_historical_entries(&db).unwrap();
    assert!(initial_history_count > 0);

    // Close the database before CLI command
    drop(db);

    // Parse CLI command and execute
    Cli::parse_from([
        "katana",
        "db",
        "--path",
        db_path.to_str().unwrap(),
        "prune",
        "keep-last-n",
        "3",
    ])
    .run()
    .expect("failed to run");

    // Reopen database to verify result
    let db_reopened = open_db_ro(&db_path.to_str().unwrap()).unwrap();

    // Should have fewer historical entries now
    let final_history_count = count_historical_entries(&db_reopened).unwrap();
    let final_headers_count = count_headers(&db_reopened).unwrap();
    assert!(final_history_count < initial_history_count);
    // assert_eq!(final_headers_count, initial_headers_
}
