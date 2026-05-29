pub mod db;
pub mod fork;

use katana_primitives::class::{ClassHash, CompiledClassHash, ContractClass};
use katana_primitives::contract::{Nonce, StorageKey, StorageValue};
use katana_primitives::state::StateUpdatesWithClasses;
use katana_primitives::{ContractAddress, Felt};
use katana_provider_api::contract::ContractClassProvider;
use katana_provider_api::state::{StateProofProvider, StateProvider, StateRootProvider};
use katana_trie::MultiProof;

use crate::ProviderResult;

#[derive(Debug)]
pub struct EmptyStateProvider;

impl StateProvider for EmptyStateProvider {
    fn class_hash_of_contract(
        &self,
        address: ContractAddress,
    ) -> ProviderResult<Option<ClassHash>> {
        let _ = address;
        Ok(None)
    }

    fn nonce(&self, address: ContractAddress) -> ProviderResult<Option<Nonce>> {
        let _ = address;
        Ok(None)
    }

    fn storage(
        &self,
        address: ContractAddress,
        storage_key: StorageKey,
    ) -> ProviderResult<Option<StorageValue>> {
        let _ = address;
        let _ = storage_key;
        Ok(None)
    }
}

impl ContractClassProvider for EmptyStateProvider {
    fn class(&self, hash: ClassHash) -> ProviderResult<Option<ContractClass>> {
        let _ = hash;
        Ok(None)
    }

    fn compiled_class_hash_of_class_hash(
        &self,
        hash: ClassHash,
    ) -> ProviderResult<Option<CompiledClassHash>> {
        let _ = hash;
        Ok(None)
    }
}

impl StateProofProvider for EmptyStateProvider {
    fn class_multiproof(&self, classes: Vec<ClassHash>) -> ProviderResult<MultiProof> {
        let _ = classes;
        Ok(MultiProof(Default::default()))
    }

    fn contract_multiproof(&self, addresses: Vec<ContractAddress>) -> ProviderResult<MultiProof> {
        let _ = addresses;
        Ok(MultiProof(Default::default()))
    }

    fn storage_multiproof(
        &self,
        address: ContractAddress,
        key: Vec<StorageKey>,
    ) -> ProviderResult<MultiProof> {
        let _ = address;
        let _ = key;
        Ok(MultiProof(Default::default()))
    }
}

impl StateRootProvider for EmptyStateProvider {
    fn classes_root(&self) -> ProviderResult<Felt> {
        Ok(Felt::ZERO)
    }

    fn contracts_root(&self) -> ProviderResult<Felt> {
        Ok(Felt::ZERO)
    }

    fn state_root(&self) -> ProviderResult<Felt> {
        Ok(Felt::ZERO)
    }

    fn storage_root(&self, contract: ContractAddress) -> ProviderResult<Option<Felt>> {
        let _ = contract;
        Ok(None)
    }
}

/// A read-only [`StateProvider`] that serves classes, deployed contracts, nonces, and storage
/// slots from a [`StateUpdatesWithClasses`] overlay, falling through to an inner provider for
/// anything not present in the overlay.
///
/// Built for the rollup genesis flow, where the STRK fee token is pre-allocated into state before
/// the genesis transactions execute (the transactions then `transfer` from this contract). Proof
/// and state-root queries are delegated to the inner provider unchanged — overlays are not
/// reflected in the trie. Callers commit the merged state afterwards through the usual provider
/// write path, which builds the tries correctly.
#[derive(Debug)]
pub struct PreloadedStateProvider<P> {
    inner: P,
    overlay: StateUpdatesWithClasses,
}

impl<P> PreloadedStateProvider<P> {
    pub fn new(inner: P, overlay: StateUpdatesWithClasses) -> Self {
        Self { inner, overlay }
    }
}

impl<P: StateProvider> StateProvider for PreloadedStateProvider<P> {
    fn class_hash_of_contract(
        &self,
        address: ContractAddress,
    ) -> ProviderResult<Option<ClassHash>> {
        if let Some(hash) = self.overlay.state_updates.deployed_contracts.get(&address) {
            return Ok(Some(*hash));
        }
        self.inner.class_hash_of_contract(address)
    }

    fn nonce(&self, address: ContractAddress) -> ProviderResult<Option<Nonce>> {
        if let Some(nonce) = self.overlay.state_updates.nonce_updates.get(&address) {
            return Ok(Some(*nonce));
        }
        self.inner.nonce(address)
    }

    fn storage(
        &self,
        address: ContractAddress,
        storage_key: StorageKey,
    ) -> ProviderResult<Option<StorageValue>> {
        if let Some(slots) = self.overlay.state_updates.storage_updates.get(&address) {
            if let Some(value) = slots.get(&storage_key) {
                return Ok(Some(*value));
            }
        }
        self.inner.storage(address, storage_key)
    }
}

impl<P: ContractClassProvider> ContractClassProvider for PreloadedStateProvider<P> {
    fn class(&self, hash: ClassHash) -> ProviderResult<Option<ContractClass>> {
        if let Some(class) = self.overlay.classes.get(&hash) {
            return Ok(Some(class.clone()));
        }
        self.inner.class(hash)
    }

    fn compiled_class_hash_of_class_hash(
        &self,
        hash: ClassHash,
    ) -> ProviderResult<Option<CompiledClassHash>> {
        // Sierra (non-legacy) declared classes carry their compiled hash in the overlay; legacy
        // declares are recorded in `deprecated_declared_classes` and have no compiled hash to
        // surface here.
        if let Some(compiled) = self.overlay.state_updates.declared_classes.get(&hash) {
            return Ok(Some(*compiled));
        }
        self.inner.compiled_class_hash_of_class_hash(hash)
    }
}

impl<P: StateProofProvider> StateProofProvider for PreloadedStateProvider<P> {
    fn class_multiproof(&self, classes: Vec<ClassHash>) -> ProviderResult<MultiProof> {
        self.inner.class_multiproof(classes)
    }

    fn contract_multiproof(&self, addresses: Vec<ContractAddress>) -> ProviderResult<MultiProof> {
        self.inner.contract_multiproof(addresses)
    }

    fn storage_multiproof(
        &self,
        address: ContractAddress,
        key: Vec<StorageKey>,
    ) -> ProviderResult<MultiProof> {
        self.inner.storage_multiproof(address, key)
    }
}

impl<P: StateRootProvider> StateRootProvider for PreloadedStateProvider<P> {
    fn classes_root(&self) -> ProviderResult<Felt> {
        self.inner.classes_root()
    }

    fn contracts_root(&self) -> ProviderResult<Felt> {
        self.inner.contracts_root()
    }

    fn state_root(&self) -> ProviderResult<Felt> {
        self.inner.state_root()
    }

    fn storage_root(&self, contract: ContractAddress) -> ProviderResult<Option<Felt>> {
        self.inner.storage_root(contract)
    }
}
