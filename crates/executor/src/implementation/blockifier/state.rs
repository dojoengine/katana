use std::collections::HashMap;
use std::fmt::Debug;
use std::ops::Deref;
use std::sync::Arc;

use blockifier::execution::contract_class::RunnableCompiledClass;
use blockifier::state::cached_state::{self, MutRefState};
use blockifier::state::errors::StateError;
use blockifier::state::state_api::{StateReader, StateResult};
use katana_primitives::class::{self, ContractClass};
use katana_primitives::Felt;
use katana_provider::error::ProviderError;
use katana_provider::traits::contract::ContractClassProvider;
use katana_provider::traits::state::{StateProofProvider, StateProvider, StateRootProvider};
use katana_provider::ProviderResult;
use parking_lot::Mutex;
use starknet_api::core::{ClassHash, CompiledClassHash, Nonce};
use starknet_api::state::StorageKey;
use tracing::trace;

use super::cache::ClassCache;
use super::utils::{self};

#[derive(Debug, Clone)]
pub struct CachedState<'a> {
    pub inner: Arc<Mutex<CachedStateInner<'a>>>,
}

#[derive(Debug)]
pub struct CachedStateInner<'a> {
    pub cached_state: cached_state::CachedState<StateProviderDb<'a>>,
    pub(super) declared_classes: HashMap<class::ClassHash, ContractClass>,
}

impl<'a> CachedState<'a> {
    pub fn new(state: impl StateProvider + 'a, class_cache: ClassCache) -> Self {
        let state = StateProviderDb::new_with_class_cache(Box::new(state), class_cache);
        let cached_state = cached_state::CachedState::new(state);

        let declared_classes = HashMap::new();
        let inner = CachedStateInner { cached_state, declared_classes };

        Self { inner: Arc::new(Mutex::new(inner)) }
    }

    pub fn with_mut_cached_state<F, R>(&self, f: F) -> R
    where
        F: FnOnce(
            &mut cached_state::CachedState<
                MutRefState<'_, cached_state::CachedState<StateProviderDb<'a>>>,
            >,
        ) -> R,
    {
        let mut inner = self.inner.lock();
        let mut state = cached_state::CachedState::new(MutRefState::new(&mut inner.cached_state));
        let result = f(&mut state);
        state.commit();
        result
    }

    pub fn with_cached_state_and_declared_classes<F, R>(&self, f: F) -> R
    where
        F: FnOnce(
            &mut cached_state::CachedState<StateProviderDb<'a>>,
            &mut HashMap<class::ClassHash, ContractClass>,
        ) -> R,
    {
        let mut inner = self.inner.lock();
        let CachedStateInner { ref mut cached_state, ref mut declared_classes } = &mut *inner;
        f(cached_state, declared_classes)
    }
}

impl ContractClassProvider for CachedState<'_> {
    fn class(
        &self,
        hash: katana_primitives::class::ClassHash,
    ) -> ProviderResult<Option<ContractClass>> {
        let state = self.inner.lock();
        if let Some(class) = state.declared_classes.get(&hash) {
            Ok(Some(class.clone()))
        } else {
            state.cached_state.state.class(hash)
        }
    }

    fn compiled_class_hash_of_class_hash(
        &self,
        hash: katana_primitives::class::ClassHash,
    ) -> ProviderResult<Option<katana_primitives::class::CompiledClassHash>> {
        let Ok(hash) = self.inner.lock().cached_state.get_compiled_class_hash(ClassHash(hash))
        else {
            return Ok(None);
        };

        if hash.0 == Felt::ZERO {
            Ok(None)
        } else {
            Ok(Some(hash.0))
        }
    }
}

impl StateProvider for CachedState<'_> {
    fn class_hash_of_contract(
        &self,
        address: katana_primitives::contract::ContractAddress,
    ) -> ProviderResult<Option<katana_primitives::class::ClassHash>> {
        let Ok(hash) =
            self.inner.lock().cached_state.get_class_hash_at(utils::to_blk_address(address))
        else {
            return Ok(None);
        };

        if hash.0 == Felt::ZERO {
            Ok(None)
        } else {
            Ok(Some(hash.0))
        }
    }

    fn nonce(
        &self,
        address: katana_primitives::contract::ContractAddress,
    ) -> ProviderResult<Option<katana_primitives::contract::Nonce>> {
        // check if the contract is deployed
        if self.class_hash_of_contract(address)?.is_none() {
            return Ok(None);
        }

        match self.inner.lock().cached_state.get_nonce_at(utils::to_blk_address(address)) {
            Ok(nonce) => Ok(Some(nonce.0)),
            Err(e) => Err(ProviderError::Other(e.to_string())),
        }
    }

    fn storage(
        &self,
        address: katana_primitives::contract::ContractAddress,
        storage_key: katana_primitives::contract::StorageKey,
    ) -> ProviderResult<Option<katana_primitives::contract::StorageValue>> {
        // check if the contract is deployed
        if self.class_hash_of_contract(address)?.is_none() {
            return Ok(None);
        }

        let address = utils::to_blk_address(address);
        let key = StorageKey(storage_key.try_into().unwrap());

        match self.inner.lock().cached_state.get_storage_at(address, key) {
            Ok(value) => Ok(Some(value)),
            Err(e) => Err(ProviderError::Other(e.to_string())),
        }
    }
}

impl StateProofProvider for CachedState<'_> {}
impl StateRootProvider for CachedState<'_> {}

pub struct StateProviderDb<'a> {
    provider: Box<dyn StateProvider + 'a>,
    compiled_class_cache: ClassCache,
}

impl Debug for StateProviderDb<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StateProviderDb")
            .field("provider", &"..")
            .field("compiled_class_cache", &self.compiled_class_cache)
            .finish()
    }
}

impl<'a> Deref for StateProviderDb<'a> {
    type Target = Box<dyn StateProvider + 'a>;

    fn deref(&self) -> &Self::Target {
        &self.provider
    }
}

impl<'a> StateProviderDb<'a> {
    /// Creates a new [`StateProviderDb`].
    pub fn new(provider: Box<dyn StateProvider + 'a>) -> Self {
        let compiled_class_cache = ClassCache::new().expect("failed to build class cache");
        Self { provider, compiled_class_cache }
    }

    /// Creates a new [`StateProviderDb`] with a custom class cache.
    pub fn new_with_class_cache(
        provider: Box<dyn StateProvider + 'a>,
        class_cache: ClassCache,
    ) -> Self {
        Self { provider, compiled_class_cache: class_cache }
    }
}

impl StateReader for StateProviderDb<'_> {
    fn get_class_hash_at(
        &self,
        contract_address: starknet_api::core::ContractAddress,
    ) -> StateResult<starknet_api::core::ClassHash> {
        self.provider
            .class_hash_of_contract(utils::to_address(contract_address))
            .map(|v| ClassHash(v.unwrap_or_default()))
            .map_err(|e| StateError::StateReadError(e.to_string()))
    }

    fn get_compiled_class_hash(
        &self,
        class_hash: starknet_api::core::ClassHash,
    ) -> StateResult<starknet_api::core::CompiledClassHash> {
        if let Some(hash) = self
            .provider
            .compiled_class_hash_of_class_hash(class_hash.0)
            .map_err(|e| StateError::StateReadError(e.to_string()))?
        {
            Ok(CompiledClassHash(hash))
        } else {
            Err(StateError::UndeclaredClassHash(class_hash))
        }
    }

    fn get_compiled_class(&self, class_hash: ClassHash) -> StateResult<RunnableCompiledClass> {
        if let Some(class) = self.compiled_class_cache.get(&class_hash.0) {
            trace!(target: "class_cache", class = format!("{}", class_hash.to_hex_string()), "Cache hit");
            return Ok(class);
        }

        trace!(target: "class_cache", class = format!("{}", class_hash.to_hex_string()), "Cache miss");

        if let Some(class) =
            self.class(class_hash.0).map_err(|e| StateError::StateReadError(e.to_string()))?
        {
            let compiled = self.compiled_class_cache.insert(class_hash.0, class);
            return Ok(compiled);
        }

        Err(StateError::UndeclaredClassHash(class_hash))
    }

    fn get_nonce_at(
        &self,
        contract_address: starknet_api::core::ContractAddress,
    ) -> StateResult<starknet_api::core::Nonce> {
        self.provider
            .nonce(utils::to_address(contract_address))
            .map(|n| Nonce(n.unwrap_or_default()))
            .map_err(|e| StateError::StateReadError(e.to_string()))
    }

    fn get_storage_at(
        &self,
        contract_address: starknet_api::core::ContractAddress,
        key: starknet_api::state::StorageKey,
    ) -> StateResult<starknet_api::hash::StarkHash> {
        self.storage(utils::to_address(contract_address), *key.0.key())
            .map(|v| v.unwrap_or_default())
            .map_err(|e| StateError::StateReadError(e.to_string()))
    }
}
