use std::collections::{BTreeMap, BTreeSet};

use katana_primitives::block::{Block, BlockHash, FinalityStatus};
use katana_primitives::class::{ClassHash, CompiledClassHash, ContractClass};
use katana_primitives::contract::{ContractAddress, Nonce, StorageKey, StorageValue};
use katana_primitives::state::{StateUpdates, StateUpdatesWithClasses};
use katana_provider::api::block::BlockWriter;
use katana_provider::api::trie::TrieWriter;
use katana_provider::{DbProviderFactory, MutableProvider, ProviderFactory};
use katana_utils::arbitrary;
use rstest::*;
use tempfile::TempDir;

#[derive(Debug)]
pub struct TempDb {
    temp_dir: TempDir,
}

impl TempDb {
    pub fn new() -> Self {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        katana_db::Db::new(temp_dir.path()).expect("failed to initialize database");
        Self { temp_dir }
    }

    pub fn provider_factory(&self) -> DbProviderFactory {
        DbProviderFactory::new(self.open_rw())
    }

    pub fn path_str(&self) -> &str {
        self.temp_dir.path().to_str().unwrap()
    }

    fn open_rw(&self) -> katana_db::Db {
        katana::cli::db::open_db_rw(self.path_str()).unwrap()
    }
}

impl Default for TempDb {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper to create an empty temporary database
#[fixture]
pub fn empty_db() -> TempDb {
    TempDb::new()
}

/// Helper to create a temporary database with arbitrary generated data
#[fixture]
pub fn db() -> TempDb {
    let db = TempDb::new();
    populate_db(&db);
    db
}

/// Populate database with test data using the TrieWriter trait
fn populate_db(db: &TempDb) {
    let provider_factory = db.provider_factory();
    let provider = provider_factory.provider_mut();

    for num in 0..=15u64 {
        let mut classes = BTreeMap::new();

        let mut declared_classes = BTreeMap::new();
        for _ in 0..10 {
            let hash = arbitrary!(ClassHash);
            declared_classes.insert(hash, arbitrary!(CompiledClassHash));
            classes.insert(hash, ContractClass::Legacy(Default::default()));
        }

        let mut deprecated_declared_classes = BTreeSet::new();
        for _ in 0..10 {
            let hash = arbitrary!(ClassHash);
            deprecated_declared_classes.insert(hash);
            classes.insert(hash, ContractClass::Legacy(Default::default()));
        }

        let mut nonce_updates = BTreeMap::new();
        for _ in 0..10 {
            nonce_updates.insert(arbitrary!(ContractAddress), arbitrary!(Nonce));
        }

        let mut storage_updates = BTreeMap::new();

        for _ in 0..10 {
            let mut storage_entries = BTreeMap::new();
            for _ in 0..10 {
                storage_entries.insert(arbitrary!(StorageKey), arbitrary!(StorageValue));
            }
            storage_updates.insert(arbitrary!(ContractAddress), storage_entries);
        }

        let mut deployed_contracts = BTreeMap::new();
        let mut replaced_classes = BTreeMap::new();

        // this is to ensure that the contract addresses in replaced_classes exist ie is deployed.
        for _ in 0..10 {
            let address = arbitrary!(ContractAddress);
            deployed_contracts.insert(address, arbitrary!(ClassHash));
            replaced_classes.insert(address, arbitrary!(ClassHash));
        }

        let state_updates = StateUpdates {
            nonce_updates,
            storage_updates,
            declared_classes,
            replaced_classes,
            deployed_contracts,
            deprecated_declared_classes,
        };

        provider.trie_insert_declared_classes(num, &state_updates.declared_classes).unwrap();
        provider.trie_insert_contract_updates(num, &state_updates).unwrap();

        let mut block = Block::default();
        block.header.number = num;

        let status = FinalityStatus::AcceptedOnL2;
        let block = block.seal_with_hash_and_status(arbitrary!(BlockHash), status);

        provider
            .insert_block_with_states_and_receipts(
                block,
                StateUpdatesWithClasses { state_updates, classes },
                Vec::new(),
                Vec::new(),
            )
            .unwrap();
    }

    provider.commit().expect("failed to commit");
}
