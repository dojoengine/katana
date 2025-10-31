use std::collections::{BTreeMap, HashMap};
use std::ops::{Range, RangeInclusive};
use std::sync::Arc;

use katana_db::abstraction::Database;
use katana_db::models::block::StoredBlockBodyIndices;
use katana_primitives::block::{
    Block, BlockHash, BlockHashOrNumber, BlockIdOrTag, BlockNumber, BlockWithTxHashes,
    FinalityStatus, Header, SealedBlockWithStatus,
};
use katana_primitives::class::{ClassHash, CompiledClassHash, ContractClass};
use katana_primitives::contract::{ContractAddress, Nonce, StorageKey, StorageValue};
use katana_primitives::env::BlockEnv;
use katana_primitives::execution::TypedTransactionExecutionInfo;
use katana_primitives::receipt::Receipt;
use katana_primitives::state::{StateUpdates, StateUpdatesWithClasses};
use katana_primitives::transaction::{TxHash, TxNumber, TxWithHash};
use katana_provider_api::block::{
    BlockHashProvider, BlockNumberProvider, BlockProvider, BlockStatusProvider, BlockWriter,
    HeaderProvider,
};
use katana_provider_api::contract::{ContractClassProvider, ContractClassWriter};
use katana_provider_api::env::BlockEnvProvider;
use katana_provider_api::stage::StageCheckpointProvider;
use katana_provider_api::state::{
    StateFactoryProvider, StateProofProvider, StateProvider, StateRootProvider, StateWriter,
};
use katana_provider_api::state_update::StateUpdateProvider;
use katana_provider_api::transaction::{
    ReceiptProvider, TransactionProvider, TransactionStatusProvider, TransactionTraceProvider,
    TransactionsProviderExt,
};
use parking_lot::RwLock;

use crate::providers::fork::state::HistoricalStateProvider as ForkHistoricalStateProvider;
use crate::providers::fork::ForkedProvider;
use crate::ProviderResult;

/// Inner cache data protected by a single lock for consistent snapshots.
#[derive(Debug, Default)]
struct StateCacheInner {
    /// Cache for contract nonces: ContractAddress -> Nonce
    nonces: HashMap<ContractAddress, Nonce>,
    /// Cache for storage values: (ContractAddress, StorageKey) -> StorageValue
    storage: HashMap<(ContractAddress, StorageKey), StorageValue>,
    /// Cache for contract class hashes: ContractAddress -> ClassHash
    class_hashes: HashMap<ContractAddress, ClassHash>,
    /// Cache for contract classes: ClassHash -> ContractClass
    classes: HashMap<ClassHash, ContractClass>,
    /// Cache for compiled class hashes: ClassHash -> CompiledClassHash
    compiled_class_hashes: HashMap<ClassHash, CompiledClassHash>,
}

/// A cache for storing state data in memory.
///
/// Uses a single read-write lock to ensure consistent snapshots across all cached data.
/// This prevents reading inconsistent state that could occur with multiple independent locks.
#[derive(Debug, Clone)]
pub struct StateCache {
    inner: Arc<RwLock<StateCacheInner>>,
}

impl Default for StateCache {
    fn default() -> Self {
        Self::new()
    }
}

impl StateCache {
    fn new() -> Self {
        Self { inner: Arc::new(RwLock::new(StateCacheInner::default())) }
    }

    fn get_nonce(&self, address: ContractAddress) -> Option<Nonce> {
        self.inner.read().nonces.get(&address).copied()
    }

    fn get_storage(&self, address: ContractAddress, key: StorageKey) -> Option<StorageValue> {
        self.inner.read().storage.get(&(address, key)).copied()
    }

    fn get_class_hash(&self, address: ContractAddress) -> Option<ClassHash> {
        self.inner.read().class_hashes.get(&address).copied()
    }

    fn get_class(&self, hash: ClassHash) -> Option<ContractClass> {
        self.inner.read().classes.get(&hash).cloned()
    }

    fn get_compiled_class_hash(&self, hash: ClassHash) -> Option<CompiledClassHash> {
        self.inner.read().compiled_class_hashes.get(&hash).copied()
    }

    /// Clears all cached data.
    pub fn clear(&self) {
        let mut cache = self.inner.write();
        cache.nonces.clear();
        cache.storage.clear();
        cache.class_hashes.clear();
        cache.classes.clear();
        cache.compiled_class_hashes.clear();
    }
}

/// A cached version of provider that wraps the underlying provider with an in-memory cache
/// for state data.
///
/// The cache is used to store frequently accessed state information such as nonces, storage values,
/// class hashes, and contract classes. When querying state through the [`StateProvider`] interface,
/// the cache is checked first before falling back to the underlying database.
#[derive(Debug, Clone)]
pub struct CachedDbProvider<Db: Database> {
    /// The underlying provider
    inner: ForkedProvider<Db>,
    /// The in-memory cache for state data
    cache: StateCache,
}

impl<Db: Database> CachedDbProvider<Db> {
    /// Creates a new [`CachedDbProvider`] wrapping the given [`ForkedProvider`].
    pub fn new(
        db: Db,
        block_id: BlockIdOrTag,
        starknet_client: katana_rpc_client::starknet::Client,
    ) -> Self {
        let inner = ForkedProvider::new(db, block_id, starknet_client);
        Self { inner, cache: StateCache::new() }
    }

    /// Returns a reference to the underlying [`ForkedProvider`].
    pub fn inner(&self) -> &ForkedProvider<Db> {
        &self.inner
    }
}

impl<Db: Database> CachedDbProvider<Db> {
    /// Returns a reference to the cache.
    pub fn cache(&self) -> &StateCache {
        &self.cache
    }

    /// Clears all cached data.
    pub fn clear_cache(&self) {
        self.cache.clear();
    }

    /// Merges state updates into the cache.
    pub fn merge_state_updates(&self, updates: &StateUpdatesWithClasses) {
        let mut cache = self.cache.inner.write();
        let state = &updates.state_updates;

        for (address, nonce) in &state.nonce_updates {
            cache.nonces.insert(*address, *nonce);
        }

        for (address, storage) in &state.storage_updates {
            for (key, value) in storage {
                cache.storage.insert((*address, *key), *value);
            }
        }

        for (address, class_hash) in &state.deployed_contracts {
            cache.class_hashes.insert(*address, *class_hash);
        }

        for (address, class_hash) in &state.replaced_classes {
            cache.class_hashes.insert(*address, *class_hash);
        }

        for (class_hash, compiled_hash) in &state.declared_classes {
            cache.compiled_class_hashes.insert(*class_hash, *compiled_hash);
        }

        for (class_hash, class) in &updates.classes {
            cache.classes.insert(*class_hash, class.clone());
        }
    }
}

impl<Db: Database + 'static> StateFactoryProvider for CachedDbProvider<Db> {
    fn latest(&self) -> ProviderResult<Box<dyn StateProvider>> {
        Ok(Box::new(CachedStateProvider { state: self.inner.latest()?, cache: self.cache.clone() }))
    }

    fn historical(
        &self,
        block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<Box<dyn StateProvider>>> {
        if let Some(state) = self.inner.historical(block_id)? {
            Ok(Some(Box::new(CachedStateProvider { state, cache: self.cache.clone() })))
        } else {
            Ok(None)
        }
    }
}

impl<Db: Database> BlockNumberProvider for CachedDbProvider<Db> {
    fn block_number_by_hash(&self, hash: BlockHash) -> ProviderResult<Option<BlockNumber>> {
        self.inner.block_number_by_hash(hash)
    }

    fn latest_number(&self) -> ProviderResult<BlockNumber> {
        self.inner.latest_number()
    }
}

impl<Db: Database> BlockHashProvider for CachedDbProvider<Db> {
    fn latest_hash(&self) -> ProviderResult<BlockHash> {
        self.inner.latest_hash()
    }

    fn block_hash_by_num(&self, num: BlockNumber) -> ProviderResult<Option<BlockHash>> {
        self.inner.block_hash_by_num(num)
    }
}

impl<Db: Database> HeaderProvider for CachedDbProvider<Db> {
    fn header(&self, id: BlockHashOrNumber) -> ProviderResult<Option<Header>> {
        self.inner.header(id)
    }
}

impl<Db: Database> BlockProvider for CachedDbProvider<Db> {
    fn block_body_indices(
        &self,
        id: BlockHashOrNumber,
    ) -> ProviderResult<Option<StoredBlockBodyIndices>> {
        self.inner.block_body_indices(id)
    }

    fn block(&self, id: BlockHashOrNumber) -> ProviderResult<Option<Block>> {
        self.inner.block(id)
    }

    fn block_with_tx_hashes(
        &self,
        id: BlockHashOrNumber,
    ) -> ProviderResult<Option<BlockWithTxHashes>> {
        self.inner.block_with_tx_hashes(id)
    }

    fn blocks_in_range(&self, range: RangeInclusive<u64>) -> ProviderResult<Vec<Block>> {
        self.inner.blocks_in_range(range)
    }
}

impl<Db: Database> BlockStatusProvider for CachedDbProvider<Db> {
    fn block_status(&self, id: BlockHashOrNumber) -> ProviderResult<Option<FinalityStatus>> {
        self.inner.block_status(id)
    }
}

impl<Db: Database> StateUpdateProvider for CachedDbProvider<Db> {
    fn state_update(&self, block_id: BlockHashOrNumber) -> ProviderResult<Option<StateUpdates>> {
        self.inner.state_update(block_id)
    }

    fn declared_classes(
        &self,
        block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<BTreeMap<ClassHash, CompiledClassHash>>> {
        self.inner.declared_classes(block_id)
    }

    fn deployed_contracts(
        &self,
        block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<BTreeMap<ContractAddress, ClassHash>>> {
        self.inner.deployed_contracts(block_id)
    }
}

impl<Db: Database> TransactionProvider for CachedDbProvider<Db> {
    fn transaction_by_hash(&self, hash: TxHash) -> ProviderResult<Option<TxWithHash>> {
        self.inner.transaction_by_hash(hash)
    }

    fn transactions_by_block(
        &self,
        block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<Vec<TxWithHash>>> {
        self.inner.transactions_by_block(block_id)
    }

    fn transaction_in_range(&self, range: Range<TxNumber>) -> ProviderResult<Vec<TxWithHash>> {
        self.inner.transaction_in_range(range)
    }

    fn transaction_block_num_and_hash(
        &self,
        hash: TxHash,
    ) -> ProviderResult<Option<(BlockNumber, BlockHash)>> {
        self.inner.transaction_block_num_and_hash(hash)
    }

    fn transaction_by_block_and_idx(
        &self,
        block_id: BlockHashOrNumber,
        idx: u64,
    ) -> ProviderResult<Option<TxWithHash>> {
        self.inner.transaction_by_block_and_idx(block_id, idx)
    }

    fn transaction_count_by_block(
        &self,
        block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<u64>> {
        self.inner.transaction_count_by_block(block_id)
    }
}

impl<Db: Database> TransactionsProviderExt for CachedDbProvider<Db> {
    fn transaction_hashes_in_range(&self, range: Range<TxNumber>) -> ProviderResult<Vec<TxHash>> {
        self.inner.transaction_hashes_in_range(range)
    }

    fn total_transactions(&self) -> ProviderResult<usize> {
        self.inner.total_transactions()
    }
}

impl<Db: Database> TransactionStatusProvider for CachedDbProvider<Db> {
    fn transaction_status(&self, hash: TxHash) -> ProviderResult<Option<FinalityStatus>> {
        self.inner.transaction_status(hash)
    }
}

impl<Db: Database> TransactionTraceProvider for CachedDbProvider<Db> {
    fn transaction_execution(
        &self,
        hash: TxHash,
    ) -> ProviderResult<Option<TypedTransactionExecutionInfo>> {
        self.inner.transaction_execution(hash)
    }

    fn transaction_executions_by_block(
        &self,
        block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<Vec<TypedTransactionExecutionInfo>>> {
        self.inner.transaction_executions_by_block(block_id)
    }

    fn transaction_executions_in_range(
        &self,
        range: Range<TxNumber>,
    ) -> ProviderResult<Vec<TypedTransactionExecutionInfo>> {
        self.inner.transaction_executions_in_range(range)
    }
}

impl<Db: Database> ReceiptProvider for CachedDbProvider<Db> {
    fn receipt_by_hash(&self, hash: TxHash) -> ProviderResult<Option<Receipt>> {
        self.inner.receipt_by_hash(hash)
    }

    fn receipts_by_block(
        &self,
        block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<Vec<Receipt>>> {
        self.inner.receipts_by_block(block_id)
    }
}

impl<Db: Database> BlockEnvProvider for CachedDbProvider<Db> {
    fn block_env_at(&self, block_id: BlockHashOrNumber) -> ProviderResult<Option<BlockEnv>> {
        self.inner.block_env_at(block_id)
    }
}

impl<Db: Database> BlockWriter for CachedDbProvider<Db> {
    fn insert_block_with_states_and_receipts(
        &self,
        block: SealedBlockWithStatus,
        states: StateUpdatesWithClasses,
        receipts: Vec<Receipt>,
        executions: Vec<TypedTransactionExecutionInfo>,
    ) -> ProviderResult<()> {
        self.inner.insert_block_with_states_and_receipts(block, states, receipts, executions)
    }
}

impl<Db: Database> StageCheckpointProvider for CachedDbProvider<Db> {
    fn checkpoint(&self, id: &str) -> ProviderResult<Option<BlockNumber>> {
        self.inner.checkpoint(id)
    }

    fn set_checkpoint(&self, id: &str, block_number: BlockNumber) -> ProviderResult<()> {
        self.inner.set_checkpoint(id, block_number)
    }
}

impl<Db: Database> StateWriter for CachedDbProvider<Db> {
    fn set_nonce(&self, address: ContractAddress, nonce: Nonce) -> ProviderResult<()> {
        self.inner.set_nonce(address, nonce)
    }

    fn set_storage(
        &self,
        address: ContractAddress,
        storage_key: StorageKey,
        storage_value: StorageValue,
    ) -> ProviderResult<()> {
        self.inner.set_storage(address, storage_key, storage_value)
    }

    fn set_class_hash_of_contract(
        &self,
        address: ContractAddress,
        class_hash: ClassHash,
    ) -> ProviderResult<()> {
        self.inner.set_class_hash_of_contract(address, class_hash)
    }
}

impl<Db: Database> ContractClassWriter for CachedDbProvider<Db> {
    fn set_class(&self, hash: ClassHash, class: ContractClass) -> ProviderResult<()> {
        self.inner.set_class(hash, class)
    }

    fn set_compiled_class_hash_of_class_hash(
        &self,
        hash: ClassHash,
        compiled_hash: CompiledClassHash,
    ) -> ProviderResult<()> {
        self.inner.set_compiled_class_hash_of_class_hash(hash, compiled_hash)
    }
}

/// A cached version of fork [`LatestStateProvider`] that checks the cache before querying the
/// database.
#[derive(Debug)]
struct CachedStateProvider<S: StateProvider> {
    state: S,
    cache: StateCache,
}

impl<S: StateProvider> ContractClassProvider for CachedStateProvider<S> {
    fn class(&self, hash: ClassHash) -> ProviderResult<Option<ContractClass>> {
        if let Some(class) = self.cache.get_class(hash) {
            return Ok(Some(class));
        }

        let class = self.state.class(hash)?;
        if let Some(ref c) = &class {
            self.cache.inner.write().classes.insert(hash, c.clone());
        }
        Ok(class)
    }

    fn compiled_class_hash_of_class_hash(
        &self,
        hash: ClassHash,
    ) -> ProviderResult<Option<CompiledClassHash>> {
        if let Some(compiled_hash) = self.cache.get_compiled_class_hash(hash) {
            return Ok(Some(compiled_hash));
        }

        let compiled_hash = self.state.compiled_class_hash_of_class_hash(hash)?;
        if let Some(ch) = compiled_hash {
            self.cache.inner.write().compiled_class_hashes.insert(hash, ch);
        }
        Ok(compiled_hash)
    }
}

impl<S: StateProvider> StateProvider for CachedStateProvider<S> {
    fn nonce(&self, address: ContractAddress) -> ProviderResult<Option<Nonce>> {
        if let Some(nonce) = self.cache.get_nonce(address) {
            Ok(Some(nonce))
        } else {
            Ok(self.state.nonce(address)?)
        }
    }

    fn storage(
        &self,
        address: ContractAddress,
        storage_key: StorageKey,
    ) -> ProviderResult<Option<StorageValue>> {
        if let Some(value) = self.cache.get_storage(address, storage_key) {
            Ok(Some(value))
        } else {
            Ok(self.state.storage(address, storage_key)?)
        }
    }

    fn class_hash_of_contract(
        &self,
        address: ContractAddress,
    ) -> ProviderResult<Option<ClassHash>> {
        if let Some(class_hash) = self.cache.get_class_hash(address) {
            return Ok(Some(class_hash));
        }

        let class_hash = self.state.class_hash_of_contract(address)?;
        if let Some(ch) = class_hash {
            self.cache.inner.write().class_hashes.insert(address, ch);
        }
        Ok(class_hash)
    }
}

/// A cached version of fork [`HistoricalStateProvider`] that checks the cache before querying the
/// database.
#[derive(Debug)]
struct CachedHistoricalStateProvider<Db: Database> {
    inner: ForkHistoricalStateProvider<Db>,
    cache: StateCache,
}

impl<Db: Database> ContractClassProvider for CachedHistoricalStateProvider<Db> {
    fn class(&self, hash: ClassHash) -> ProviderResult<Option<ContractClass>> {
        if let Some(class) = self.cache.get_class(hash) {
            Ok(Some(class))
        } else {
            Ok(self.inner.class(hash)?)
        }
    }

    fn compiled_class_hash_of_class_hash(
        &self,
        hash: ClassHash,
    ) -> ProviderResult<Option<CompiledClassHash>> {
        if let Some(compiled_hash) = self.cache.get_compiled_class_hash(hash) {
            Ok(Some(compiled_hash))
        } else {
            Ok(self.inner.compiled_class_hash_of_class_hash(hash)?)
        }
    }
}

impl<Db: Database> StateProvider for CachedHistoricalStateProvider<Db> {
    fn nonce(&self, address: ContractAddress) -> ProviderResult<Option<Nonce>> {
        if let Some(nonce) = self.cache.get_nonce(address) {
            Ok(Some(nonce))
        } else {
            Ok(self.inner.nonce(address)?)
        }
    }

    fn storage(
        &self,
        address: ContractAddress,
        storage_key: StorageKey,
    ) -> ProviderResult<Option<StorageValue>> {
        if let Some(value) = self.cache.get_storage(address, storage_key) {
            Ok(Some(value))
        } else {
            Ok(self.inner.storage(address, storage_key)?)
        }
    }

    fn class_hash_of_contract(
        &self,
        address: ContractAddress,
    ) -> ProviderResult<Option<ClassHash>> {
        if let Some(class_hash) = self.cache.get_class_hash(address) {
            Ok(Some(class_hash))
        } else {
            Ok(self.inner.class_hash_of_contract(address)?)
        }
    }
}

impl<S: StateProvider> StateProofProvider for CachedStateProvider<S> {}
impl<S: StateProvider> StateRootProvider for CachedStateProvider<S> {}
impl<Db: Database> StateProofProvider for CachedHistoricalStateProvider<Db> {}
impl<Db: Database> StateRootProvider for CachedHistoricalStateProvider<Db> {}
