pub mod db;
#[cfg(feature = "fork")]
pub mod fork;

use katana_primitives::class::{ClassHash, CompiledClassHash, ContractClass};
use katana_primitives::contract::{Nonce, StorageKey, StorageValue};
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
