use std::cmp::Ordering;
use std::fmt;

use katana_db::abstraction::{Database, DbTxMut};
use katana_db::models::contract::{ContractClassChange, ContractNonceChange};
use katana_db::models::storage::{ContractStorageEntry, ContractStorageKey, StorageEntry};
use katana_db::tables;
use katana_fork::BackendClient;
use katana_primitives::block::{BlockHashOrNumber, BlockNumber};
use katana_primitives::class::{ClassHash, CompiledClassHash, ContractClass};
use katana_primitives::contract::{GenericContractInfo, Nonce, StorageKey, StorageValue};
use katana_primitives::{ContractAddress, Felt};

use super::db::{self};
use super::ForkedProvider;
use crate::error::ProviderError;
use crate::traits::block::BlockNumberProvider;
use crate::traits::contract::{ContractClassProvider, ContractClassWriter};
use crate::traits::state::{
    StateFactoryProvider, StateProofProvider, StateProvider, StateRootProvider, StateWriter,
};
use crate::ProviderResult;

impl<Db: Database> StateFactoryProvider for ForkedProvider<Db> {
    fn latest(&self) -> ProviderResult<Box<dyn StateProvider>> {
        let tx = self.db().tx_mut()?;
        let provider = db::state::LatestStateProvider::new(tx);
        Ok(Box::new(LatestStateProvider { backend: self.backend.clone(), provider }))
    }

    fn historical(
        &self,
        block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<Box<dyn StateProvider>>> {
        let block_number = match block_id {
            BlockHashOrNumber::Hash(hash) => self.provider.block_number_by_hash(hash)?,

            BlockHashOrNumber::Num(num) => {
                let latest_num = self.provider.latest_number()?;

                match num.cmp(&latest_num) {
                    Ordering::Less => Some(num),
                    Ordering::Greater => return Ok(None),
                    Ordering::Equal => return self.latest().map(Some),
                }
            }
        };

        let Some(block) = block_number else { return Ok(None) };
        let tx = self.db().tx_mut()?;
        let client = self.backend.clone();

        Ok(Some(Box::new(HistoricalStateProvider::new(tx, block, client))))
    }
}

#[derive(Debug)]
struct LatestStateProvider<Tx: DbTxMut> {
    backend: BackendClient,
    provider: db::state::LatestStateProvider<Tx>,
}

impl<Tx> ContractClassProvider for LatestStateProvider<Tx>
where
    Tx: DbTxMut + Send + Sync,
{
    fn class(&self, hash: ClassHash) -> ProviderResult<Option<ContractClass>> {
        if let Some(class) = self.provider.class(hash)? {
            Ok(Some(class))
        } else if let Some(class) = self.backend.get_class_at(hash)? {
            self.provider.tx().put::<tables::Classes>(hash, class.clone())?;
            Ok(Some(class))
        } else {
            Ok(None)
        }
    }

    fn compiled_class_hash_of_class_hash(
        &self,
        hash: ClassHash,
    ) -> ProviderResult<Option<CompiledClassHash>> {
        if let Some(compiled_hash) = self.provider.compiled_class_hash_of_class_hash(hash)? {
            Ok(Some(compiled_hash))
        } else if let Some(compiled_hash) = self.backend.get_compiled_class_hash(hash)? {
            self.provider.tx().put::<tables::CompiledClassHashes>(hash, compiled_hash)?;
            Ok(Some(compiled_hash))
        } else {
            Ok(None)
        }
    }
}

impl<Tx> StateProvider for LatestStateProvider<Tx>
where
    Tx: DbTxMut + fmt::Debug + Send + Sync,
{
    fn nonce(&self, address: ContractAddress) -> ProviderResult<Option<Nonce>> {
        if let Some(nonce) = self.provider.nonce(address)? {
            Ok(Some(nonce))
        } else if let Some(nonce) = self.backend.get_nonce(address)? {
            let class_hash = self
                .backend
                .get_class_hash_at(address)?
                .ok_or(ProviderError::MissingContractClassHash { address })?;

            let entry = GenericContractInfo { nonce, class_hash };
            self.provider.tx().put::<tables::ContractInfo>(address, entry)?;

            Ok(Some(nonce))
        } else {
            Ok(None)
        }
    }

    fn class_hash_of_contract(
        &self,
        address: ContractAddress,
    ) -> ProviderResult<Option<ClassHash>> {
        if let Some(class_hash) = self.provider.class_hash_of_contract(address)? {
            Ok(Some(class_hash))
        } else if let Some(class_hash) = self.backend.get_class_hash_at(address)? {
            let nonce = self
                .backend
                .get_nonce(address)?
                .ok_or(ProviderError::MissingContractNonce { address })?;

            let entry = GenericContractInfo { class_hash, nonce };
            self.provider.tx().put::<tables::ContractInfo>(address, entry)?;

            Ok(Some(class_hash))
        } else {
            Ok(None)
        }
    }

    fn storage(
        &self,
        address: ContractAddress,
        key: StorageKey,
    ) -> ProviderResult<Option<StorageValue>> {
        if let Some(value) = self.provider.storage(address, key)? {
            Ok(Some(value))
        } else if let Some(value) = self.backend.get_storage(address, key)? {
            let entry = StorageEntry { key, value };
            self.provider.tx().put::<tables::ContractStorage>(address, entry)?;
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }
}

impl<Tx> StateProofProvider for LatestStateProvider<Tx>
where
    Tx: DbTxMut + fmt::Debug + Send + Sync,
{
    fn class_multiproof(&self, classes: Vec<ClassHash>) -> ProviderResult<katana_trie::MultiProof> {
        self.provider.class_multiproof(classes)
    }

    fn contract_multiproof(
        &self,
        addresses: Vec<ContractAddress>,
    ) -> ProviderResult<katana_trie::MultiProof> {
        self.provider.contract_multiproof(addresses)
    }

    fn storage_multiproof(
        &self,
        address: ContractAddress,
        storage_keys: Vec<StorageKey>,
    ) -> ProviderResult<katana_trie::MultiProof> {
        self.provider.storage_multiproof(address, storage_keys)
    }
}

impl<Tx> StateRootProvider for LatestStateProvider<Tx>
where
    Tx: DbTxMut + fmt::Debug + Send + Sync,
{
    fn classes_root(&self) -> ProviderResult<Felt> {
        self.provider.classes_root()
    }

    fn contracts_root(&self) -> ProviderResult<Felt> {
        self.provider.contracts_root()
    }

    fn storage_root(&self, contract: ContractAddress) -> ProviderResult<Option<Felt>> {
        self.provider.storage_root(contract)
    }
}

#[derive(Debug)]
struct HistoricalStateProvider<Tx: DbTxMut> {
    backend: BackendClient,
    provider: db::state::HistoricalStateProvider<Tx>,
}

impl<Tx: DbTxMut> HistoricalStateProvider<Tx> {
    pub fn new(tx: Tx, block: BlockNumber, backend: BackendClient) -> Self {
        let provider = db::state::HistoricalStateProvider::new(tx, block);
        Self { backend, provider }
    }
}

impl<Tx> ContractClassProvider for HistoricalStateProvider<Tx>
where
    Tx: DbTxMut + Send + Sync,
{
    fn class(&self, hash: ClassHash) -> ProviderResult<Option<ContractClass>> {
        if let Some(class) = self.provider.class(hash)? {
            Ok(Some(class))
        } else if let Some(class) = self.backend.get_class_at(hash)? {
            self.provider.tx().put::<tables::Classes>(hash, class.clone())?;
            Ok(Some(class))
        } else {
            Ok(None)
        }
    }

    fn compiled_class_hash_of_class_hash(
        &self,
        hash: ClassHash,
    ) -> ProviderResult<Option<CompiledClassHash>> {
        if let Some(compiled_hash) = self.provider.compiled_class_hash_of_class_hash(hash)? {
            Ok(Some(compiled_hash))
        } else if let Some(compiled_hash) = self.backend.get_compiled_class_hash(hash)? {
            self.provider.tx().put::<tables::CompiledClassHashes>(hash, compiled_hash)?;
            Ok(Some(compiled_hash))
        } else {
            Ok(None)
        }
    }
}

impl<Tx> StateProvider for HistoricalStateProvider<Tx>
where
    Tx: DbTxMut + fmt::Debug + Send + Sync,
{
    fn nonce(&self, address: ContractAddress) -> ProviderResult<Option<Nonce>> {
        if let Some(nonce) = self.provider.nonce(address)? {
            Ok(Some(nonce))
        } else if let Some(nonce) = self.backend.get_nonce(address)? {
            let block = self.provider.block();
            let entry = ContractNonceChange { contract_address: address, nonce };

            self.provider.tx().put::<tables::NonceChangeHistory>(block, entry)?;
            Ok(Some(nonce))
        } else {
            Ok(None)
        }
    }

    fn class_hash_of_contract(
        &self,
        address: ContractAddress,
    ) -> ProviderResult<Option<ClassHash>> {
        if let Some(class_hash) = self.provider.class_hash_of_contract(address)? {
            Ok(Some(class_hash))
        } else if let Some(class_hash) = self.backend.get_class_hash_at(address)? {
            let block = self.provider.block();
            let entry = ContractClassChange { contract_address: address, class_hash };

            self.provider.tx().put::<tables::ClassChangeHistory>(block, entry)?;
            Ok(Some(class_hash))
        } else {
            Ok(None)
        }
    }

    fn storage(
        &self,
        address: ContractAddress,
        key: StorageKey,
    ) -> ProviderResult<Option<StorageValue>> {
        if let Some(value) = self.provider.storage(address, key)? {
            Ok(Some(value))
        } else if let Some(value) = self.backend.get_storage(address, key)? {
            let key = ContractStorageKey { contract_address: address, key };
            let block = self.provider.block();

            let block_list = self.provider.tx().get::<tables::StorageChangeSet>(key.clone())?;
            let mut block_list = block_list.unwrap_or_default();
            block_list.insert(block);

            self.provider.tx().put::<tables::StorageChangeSet>(key.clone(), block_list)?;
            let change_entry = ContractStorageEntry { key, value };
            self.provider.tx().put::<tables::StorageChangeHistory>(block, change_entry)?;

            Ok(Some(value))
        } else {
            Ok(None)
        }
    }
}

impl<Tx> StateProofProvider for HistoricalStateProvider<Tx>
where
    Tx: DbTxMut + fmt::Debug + Send + Sync,
{
    fn class_multiproof(&self, classes: Vec<ClassHash>) -> ProviderResult<katana_trie::MultiProof> {
        self.provider.class_multiproof(classes)
    }

    fn contract_multiproof(
        &self,
        addresses: Vec<ContractAddress>,
    ) -> ProviderResult<katana_trie::MultiProof> {
        self.provider.contract_multiproof(addresses)
    }

    fn storage_multiproof(
        &self,
        address: ContractAddress,
        storage_keys: Vec<StorageKey>,
    ) -> ProviderResult<katana_trie::MultiProof> {
        self.provider.storage_multiproof(address, storage_keys)
    }
}

impl<Tx> StateRootProvider for HistoricalStateProvider<Tx>
where
    Tx: DbTxMut + fmt::Debug + Send + Sync,
{
    fn classes_root(&self) -> ProviderResult<Felt> {
        self.provider.classes_root()
    }

    fn contracts_root(&self) -> ProviderResult<Felt> {
        self.provider.contracts_root()
    }

    fn storage_root(&self, contract: ContractAddress) -> ProviderResult<Option<Felt>> {
        self.provider.storage_root(contract)
    }
}

impl<Db: Database> StateWriter for ForkedProvider<Db> {
    fn set_class_hash_of_contract(
        &self,
        address: ContractAddress,
        class_hash: ClassHash,
    ) -> ProviderResult<()> {
        self.provider.set_class_hash_of_contract(address, class_hash)
    }

    fn set_nonce(&self, address: ContractAddress, nonce: Nonce) -> ProviderResult<()> {
        self.provider.set_nonce(address, nonce)
    }

    fn set_storage(
        &self,
        address: ContractAddress,
        storage_key: StorageKey,
        storage_value: StorageValue,
    ) -> ProviderResult<()> {
        self.provider.set_storage(address, storage_key, storage_value)
    }
}

impl<Db: Database> ContractClassWriter for ForkedProvider<Db> {
    fn set_class(&self, hash: ClassHash, class: ContractClass) -> ProviderResult<()> {
        self.provider.set_class(hash, class)
    }

    fn set_compiled_class_hash_of_class_hash(
        &self,
        hash: ClassHash,
        compiled_hash: CompiledClassHash,
    ) -> ProviderResult<()> {
        self.provider.set_compiled_class_hash_of_class_hash(hash, compiled_hash)
    }
}
