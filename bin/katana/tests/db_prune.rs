use std::path::PathBuf;

use anyhow::{Context, Result};
use arbitrary::{Arbitrary, Unstructured};
use clap::Parser;
use katana::cli::Cli;
use katana_db::abstraction::{Database, DbCursor, DbTx, DbTxMut};
use katana_db::mdbx::DbEnv;
use katana_db::models::trie::{TrieDatabaseKey, TrieDatabaseValue, TrieHistoryEntry};
use katana_db::tables::{self};
use katana_primitives::block::Header;
use katana_utils::random_bytes;
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

/// Generate arbitrary test data for a block range
fn generate_trie_entries(
    num_blocks: u64,
    entries_per_block: usize,
) -> Result<Vec<(u64, Vec<TrieHistoryEntry>)>> {
    let mut blocks_data = Vec::new();

    for block_num in 1..=num_blocks {
        let mut block_entries = Vec::new();

        // Create deterministic but pseudo-random data for each block
        for _ in 0..entries_per_block {
            let key = TrieDatabaseKey::arbitrary(&mut Unstructured::new(&random_bytes(
                TrieDatabaseKey::size_hint(0).0,
            )))
            .context("Failed to generate arbitrary key")?;

            let value = TrieDatabaseValue::from_vec(vec![1]);

            block_entries.push(TrieHistoryEntry { key, value });
        }

        blocks_data.push((block_num, block_entries));
    }

    Ok(blocks_data)
}

/// Populate database with arbitrary generated data
fn populate_db(db: &TempDb) -> Result<()> {
    db.open_rw().update(|tx| {
        for block_num in 0..=15u64 {
            tx.put::<tables::Headers>(block_num, Header::default())?;
        }

        // Insert classes trie history
        let classes_data = generate_trie_entries(15, 5)?;
        for (block_num, entries) in classes_data {
            for entry in entries {
                tx.put::<tables::ClassesTrieHistory>(block_num, entry)?;
            }
        }

        // Insert contracts trie history
        let contracts_data = generate_trie_entries(15, 3)?;
        for (block_num, entries) in contracts_data {
            for entry in entries {
                tx.put::<tables::ContractsTrieHistory>(block_num, entry)?;
            }
        }

        // Insert storages trie history
        let storages_data = generate_trie_entries(15, 7)?;
        for (block_num, entries) in storages_data {
            for entry in entries {
                tx.put::<tables::StoragesTrieHistory>(block_num, entry)?;
            }
        }

        Ok(())
    })?
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

#[rstest]
fn prune_latest_removes_all_history(db: TempDb) {
    ///////////////////////////////////////////////////////////////////////////////////
    // Verify we have historical data before pruning
    ///////////////////////////////////////////////////////////////////////////////////
    let env = db.open_ro();
    let initial_history_count = count_historical_entries(&env).unwrap();
    let initial_headers_count = count_headers(&env).unwrap();

    assert!(initial_history_count > 0);
    assert_eq!(initial_headers_count, 16);
    drop(env);
    ///////////////////////////////////////////////////////////////////////////////////

    let path = db.path_str();
    Cli::parse_from(["katana", "db", "prune", "--path", path, "latest"]).run().unwrap();

    ///////////////////////////////////////////////////////////////////////////////////
    // Verify pruning outcome
    ///////////////////////////////////////////////////////////////////////////////////
    let env = db.open_ro();
    let final_history_count = count_historical_entries(&env).unwrap();
    let final_headers_count = count_headers(&env).unwrap();

    assert_eq!(final_history_count, 0);
    assert_eq!(final_headers_count, initial_headers_count, "Headers should be unchanged");
    drop(env);
    ///////////////////////////////////////////////////////////////////////////////////
}

#[rstest]
fn prune_keep_last_n_blocks(db: TempDb) {
    ///////////////////////////////////////////////////////////////////////////////////
    // Verify we have historical data before pruning
    ///////////////////////////////////////////////////////////////////////////////////
    let env = db.open_ro();
    let initial_history_count = count_historical_entries(&env).unwrap();
    let initial_headers_count = count_headers(&env).unwrap();

    assert!(initial_history_count > 0);
    assert_eq!(initial_headers_count, 16);
    drop(env);
    ///////////////////////////////////////////////////////////////////////////////////

    let path = db.path_str();
    Cli::parse_from(["katana", "db", "prune", "--path", path, "keep-last-n", "3"]).run().unwrap();

    ///////////////////////////////////////////////////////////////////////////////////
    // Verify pruning outcome
    ///////////////////////////////////////////////////////////////////////////////////
    let env = db.open_ro();
    let final_history_count = count_historical_entries(&env).unwrap();
    let final_headers_count = count_headers(&env).unwrap();

    assert!(final_history_count < initial_history_count);
    assert_eq!(final_headers_count, initial_headers_count, "Headers should be unchanged");
    drop(env);
    ///////////////////////////////////////////////////////////////////////////////////
}
