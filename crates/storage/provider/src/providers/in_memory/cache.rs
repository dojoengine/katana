use std::collections::HashMap;
use std::sync::Arc;

use katana_db::models::block::StoredBlockBodyIndices;
use katana_primitives::block::{BlockHash, BlockNumber, FinalityStatus, Header};
use katana_primitives::class::{ClassHash, CompiledClass, CompiledClassHash, ContractClass};
use katana_primitives::contract::{ContractAddress, GenericContractInfo, StorageKey, StorageValue};
use katana_primitives::receipt::Receipt;
use katana_primitives::state::{StateUpdates, StateUpdatesWithClasses};
use katana_primitives::trace::TxExecInfo;
use katana_primitives::transaction::{Tx, TxHash, TxNumber};
use parking_lot::RwLock;

type ContractStorageMap = HashMap<ContractAddress, HashMap<StorageKey, StorageValue>>;
type ContractStateMap = HashMap<ContractAddress, GenericContractInfo>;

type ClassesMap = HashMap<ClassHash, ContractClass>;
type CompiledClassesMap = HashMap<ClassHash, CompiledClass>;
type CompiledClassHashesMap = HashMap<ClassHash, CompiledClassHash>;

#[derive(Default, Debug)]
pub struct SharedContractClasses {
    pub(crate) classes: RwLock<ClassesMap>,
    pub(crate) compiled_classes: RwLock<CompiledClassesMap>,
}

#[derive(Debug)]
pub struct CacheSnapshotWithoutClasses<Db> {
    pub(crate) db: Db,
    pub(crate) storage: ContractStorageMap,
    pub(crate) contract_state: ContractStateMap,
    pub(crate) compiled_class_hashes: CompiledClassHashesMap,
}

#[derive(Debug)]
pub struct CacheStateDb<Db> {
    pub(crate) db: Db,
    pub(crate) storage: RwLock<ContractStorageMap>,
    pub(crate) contract_state: RwLock<ContractStateMap>,
    pub(crate) shared_contract_classes: Arc<SharedContractClasses>,
    pub(crate) compiled_class_hashes: RwLock<CompiledClassHashesMap>,
}

impl<Db> CacheStateDb<Db> {
    /// Applies the given state updates to the cache.
    pub fn insert_updates(&self, updates: StateUpdatesWithClasses) {
        let mut storage = self.storage.write();
        let mut contract_state = self.contract_state.write();
        let mut compiled_class_hashes = self.compiled_class_hashes.write();
        let mut classes = self.shared_contract_classes.classes.write();
        let mut compiled_classes = self.shared_contract_classes.compiled_classes.write();

        for (contract_address, nonce) in updates.state_updates.nonce_updates {
            let info = contract_state.entry(contract_address).or_default();
            info.nonce = nonce;
        }

        for (contract_address, class_hash) in updates.state_updates.deployed_contracts {
            let info = contract_state.entry(contract_address).or_default();
            info.class_hash = class_hash;
        }

        for (contract_address, storage_changes) in updates.state_updates.storage_updates {
            let contract_storage = storage.entry(contract_address).or_default();
            contract_storage.extend(storage_changes);
        }

        for (hash, class) in updates.classes.clone() {
            let compiled = class.compile().expect("failed to compiled class");
            compiled_classes.insert(hash, compiled);
        }

        classes.extend(updates.classes);
        compiled_class_hashes.extend(updates.state_updates.declared_classes);
    }
}

#[derive(Debug)]
pub struct CacheDb<Db> {
    pub(crate) db: Db,
    pub(crate) block_headers: HashMap<BlockNumber, Header>,
    pub(crate) block_hashes: HashMap<BlockNumber, BlockHash>,
    pub(crate) block_numbers: HashMap<BlockHash, BlockNumber>,
    pub(crate) block_statusses: HashMap<BlockNumber, FinalityStatus>,
    pub(crate) block_body_indices: HashMap<BlockNumber, StoredBlockBodyIndices>,
    pub(crate) latest_block_hash: BlockHash,
    pub(crate) latest_block_number: BlockNumber,
    pub(crate) state_update: HashMap<BlockNumber, StateUpdates>,
    pub(crate) receipts: Vec<Receipt>,
    pub(crate) transactions: Vec<Tx>,
    pub(crate) transactions_executions: Vec<TxExecInfo>,
    pub(crate) transaction_hashes: HashMap<TxNumber, TxHash>,
    pub(crate) transaction_numbers: HashMap<TxHash, TxNumber>,
    pub(crate) transaction_block: HashMap<TxNumber, BlockNumber>,
}

impl<Db> CacheStateDb<Db> {
    pub fn new(db: Db) -> Self {
        Self {
            db,
            storage: RwLock::new(HashMap::new()),
            contract_state: RwLock::new(HashMap::new()),
            compiled_class_hashes: RwLock::new(HashMap::new()),
            shared_contract_classes: Arc::new(SharedContractClasses::default()),
        }
    }
}

impl<Db> CacheDb<Db> {
    pub fn new(db: Db) -> Self {
        Self {
            db,
            receipts: Vec::new(),
            transactions: Vec::new(),
            state_update: HashMap::new(),
            block_hashes: HashMap::new(),
            block_headers: HashMap::new(),
            block_numbers: HashMap::new(),
            block_statusses: HashMap::new(),
            transaction_block: HashMap::new(),
            transaction_hashes: HashMap::new(),
            block_body_indices: HashMap::new(),
            transaction_numbers: HashMap::new(),
            transactions_executions: Vec::new(),
            latest_block_hash: Default::default(),
            latest_block_number: Default::default(),
        }
    }
}

impl<Db> std::ops::Deref for CacheStateDb<Db> {
    type Target = Db;
    fn deref(&self) -> &Self::Target {
        &self.db
    }
}

impl<Db> std::ops::Deref for CacheDb<Db> {
    type Target = Db;
    fn deref(&self) -> &Self::Target {
        &self.db
    }
}
