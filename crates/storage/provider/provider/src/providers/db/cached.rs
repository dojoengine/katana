use std::collections::HashMap;
use std::sync::Arc;

use katana_primitives::class::{ClassHash, CompiledClassHash, ContractClass};
use katana_primitives::contract::{ContractAddress, Nonce, StorageKey, StorageValue};
use katana_primitives::state::StateUpdatesWithClasses;
use katana_provider_api::contract::ContractClassProvider;
use katana_provider_api::state::{StateProofProvider, StateProvider, StateRootProvider};
use parking_lot::RwLock;

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
pub struct SharedStateCache {
    inner: Arc<RwLock<StateCacheInner>>,
}

impl Default for SharedStateCache {
    fn default() -> Self {
        Self::new()
    }
}

impl SharedStateCache {
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

    /// Merges state updates into the cache.
    pub fn merge_state_updates(&self, updates: &StateUpdatesWithClasses) {
        let mut cache = self.inner.write();
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

/// A cached version of fork [`LatestStateProvider`] that checks the cache before querying the
/// database.
#[derive(Debug)]
pub struct CachedStateProvider<S: StateProvider> {
    state: S,
    cache: SharedStateCache,
}

impl<S: StateProvider> CachedStateProvider<S> {
    pub fn new(state: S, cache: SharedStateCache) -> Self {
        Self { state, cache }
    }
}

impl<S: StateProvider> ContractClassProvider for CachedStateProvider<S> {
    fn class(&self, hash: ClassHash) -> ProviderResult<Option<ContractClass>> {
        if let Some(class) = self.cache.get_class(hash) {
            return Ok(Some(class));
        } else {
            Ok(self.state.class(hash)?)
        }
    }

    fn compiled_class_hash_of_class_hash(
        &self,
        hash: ClassHash,
    ) -> ProviderResult<Option<CompiledClassHash>> {
        if let Some(compiled_hash) = self.cache.get_compiled_class_hash(hash) {
            return Ok(Some(compiled_hash));
        }

        let compiled_hash = self.state.compiled_class_hash_of_class_hash(hash)?;
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
        } else {
            Ok(self.state.class_hash_of_contract(address)?)
        }
    }
}

impl<S: StateProvider> StateProofProvider for CachedStateProvider<S> {}
impl<S: StateProvider> StateRootProvider for CachedStateProvider<S> {}
