use std::cmp::Ordering;

use katana_db::abstraction::DbTxMut;
use katana_db::models::contract::{ContractClassChange, ContractNonceChange};
use katana_db::models::storage::{ContractStorageEntry, ContractStorageKey, StorageEntry};
use katana_db::tables;
use katana_fork::BackendClient;
use katana_primitives::block::{BlockHashOrNumber, BlockNumber};
use katana_primitives::class::{ClassHash, CompiledClassHash, ContractClass};
use katana_primitives::contract::{GenericContractInfo, Nonce, StorageKey, StorageValue};
use katana_primitives::{ContractAddress, Felt};
use katana_provider_api::block::BlockNumberProvider;
use katana_provider_api::contract::{ContractClassProvider, ContractClassWriter};
use katana_provider_api::state::{
    StateFactoryProvider, StateProofProvider, StateProvider, StateRootProvider, StateWriter,
};
use katana_provider_api::ProviderError;

use super::db::{self};
use super::ForkedProvider;
use crate::ProviderResult;

impl<Tx: DbTxMut> StateFactoryProvider for ForkedProvider<Tx> {
    fn latest(&self) -> ProviderResult<Box<dyn StateProvider>> {
        let base_provider = db::state::LatestStateProvider(self.provider.tx().clone());
        let backend_client = self.backend_client.clone();
        let forked_block_id = self.block_id;
        Ok(Box::new(LatestStateProvider { forked_block_id, backend_client, base_provider }))
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

        let tx = self.provider.tx().clone();
        let backend_client = self.backend_client.clone();

        Ok(Some(Box::new(HistoricalStateProvider::new(tx, backend_client, block, self.block_id))))
    }
}

#[derive(Debug)]
struct LatestStateProvider<Tx: DbTxMut> {
    backend_client: BackendClient,
    forked_block_id: BlockNumber,
    base_provider: db::state::LatestStateProvider<Tx>,
}

impl<Tx: DbTxMut> ContractClassProvider for LatestStateProvider<Tx> {
    fn class(&self, hash: ClassHash) -> ProviderResult<Option<ContractClass>> {
        if let Some(class) = self.base_provider.class(hash)? {
            Ok(Some(class))
        } else if let Some(class) = self.backend_client.get_class_at(hash, self.forked_block_id)? {
            self.base_provider.0.put::<tables::Classes>(hash, class.clone().into())?;
            Ok(Some(class))
        } else {
            Ok(None)
        }
    }

    fn compiled_class_hash_of_class_hash(
        &self,
        hash: ClassHash,
    ) -> ProviderResult<Option<CompiledClassHash>> {
        if let res @ Some(..) = self.base_provider.compiled_class_hash_of_class_hash(hash)? {
            Ok(res)
        } else if let Some(compiled_hash) =
            self.backend_client.get_compiled_class_hash(hash, self.forked_block_id)?
        {
            self.base_provider.0.put::<tables::CompiledClassHashes>(hash, compiled_hash)?;
            Ok(Some(compiled_hash))
        } else {
            Ok(None)
        }
    }
}

impl<Tx: DbTxMut> StateProvider for LatestStateProvider<Tx> {
    fn nonce(&self, address: ContractAddress) -> ProviderResult<Option<Nonce>> {
        if let res @ Some(..) = self.base_provider.nonce(address)? {
            Ok(res)
        } else if let Some(nonce) = self.backend_client.get_nonce(address, self.forked_block_id)? {
            let class_hash = self
                .backend_client
                .get_class_hash_at(address, self.forked_block_id)?
                .ok_or(ProviderError::MissingContractClassHash { address })?;

            let entry = GenericContractInfo { nonce, class_hash };
            self.base_provider.0.put::<tables::ContractInfo>(address, entry)?;

            Ok(Some(nonce))
        } else {
            Ok(None)
        }
    }

    fn class_hash_of_contract(
        &self,
        address: ContractAddress,
    ) -> ProviderResult<Option<ClassHash>> {
        if let res @ Some(..) = self.base_provider.class_hash_of_contract(address)? {
            Ok(res)
        } else if let Some(class_hash) =
            self.backend_client.get_class_hash_at(address, self.forked_block_id)?
        {
            let nonce = self
                .backend_client
                .get_nonce(address, self.forked_block_id)?
                .ok_or(ProviderError::MissingContractNonce { address })?;

            let entry = GenericContractInfo { class_hash, nonce };
            self.base_provider.0.put::<tables::ContractInfo>(address, entry)?;

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
        if let res @ Some(..) = self.base_provider.storage(address, key)? {
            Ok(res)
        } else if let Some(value) =
            self.backend_client.get_storage(address, key, self.forked_block_id)?
        {
            let entry = StorageEntry { key, value };
            self.base_provider.0.put::<tables::ContractStorage>(address, entry)?;
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }
}

impl<Tx: DbTxMut> StateProofProvider for LatestStateProvider<Tx> {
    fn class_multiproof(&self, classes: Vec<ClassHash>) -> ProviderResult<katana_trie::MultiProof> {
        self.base_provider.class_multiproof(classes)
    }

    fn contract_multiproof(
        &self,
        addresses: Vec<ContractAddress>,
    ) -> ProviderResult<katana_trie::MultiProof> {
        self.base_provider.contract_multiproof(addresses)
    }

    fn storage_multiproof(
        &self,
        address: ContractAddress,
        storage_keys: Vec<StorageKey>,
    ) -> ProviderResult<katana_trie::MultiProof> {
        self.base_provider.storage_multiproof(address, storage_keys)
    }
}

impl<Tx: DbTxMut> StateRootProvider for LatestStateProvider<Tx> {
    fn classes_root(&self) -> ProviderResult<Felt> {
        self.base_provider.classes_root()
    }

    fn contracts_root(&self) -> ProviderResult<Felt> {
        self.base_provider.contracts_root()
    }

    fn storage_root(&self, contract: ContractAddress) -> ProviderResult<Option<Felt>> {
        self.base_provider.storage_root(contract)
    }
}

#[derive(Debug)]
struct HistoricalStateProvider<Tx: DbTxMut> {
    block_number: BlockNumber,
    forked_block_number: BlockNumber,
    backend_client: BackendClient,
    base_provider: db::state::HistoricalStateProvider<Tx>,
}

impl<Tx: DbTxMut> HistoricalStateProvider<Tx> {
    fn new(
        tx: Tx,
        backend_client: BackendClient,
        block_number: BlockNumber,
        forked_block_number: BlockNumber,
    ) -> Self {
        let base_provider = db::state::HistoricalStateProvider::new(tx, block_number);
        Self { backend_client, base_provider, block_number, forked_block_number }
    }
}

impl<Tx: DbTxMut> ContractClassProvider for HistoricalStateProvider<Tx> {
    fn class(&self, hash: ClassHash) -> ProviderResult<Option<ContractClass>> {
        if let res @ Some(..) = self.base_provider.class(hash)? {
            return Ok(res);
        }

        if self.block_number > self.forked_block_number {
            return Ok(None);
        }

        if let class @ Some(..) = self.backend_client.get_class_at(hash, self.block_number)? {
            Ok(class)
        } else {
            Ok(None)
        }
    }

    fn compiled_class_hash_of_class_hash(
        &self,
        hash: ClassHash,
    ) -> ProviderResult<Option<CompiledClassHash>> {
        if let res @ Some(..) = self.base_provider.compiled_class_hash_of_class_hash(hash)? {
            return Ok(res);
        }

        if self.block_number > self.forked_block_number {
            return Ok(None);
        }

        if let Some(compiled_hash) =
            self.backend_client.get_compiled_class_hash(hash, self.block_number)?
        {
            self.base_provider.tx().put::<tables::CompiledClassHashes>(hash, compiled_hash)?;
            Ok(Some(compiled_hash))
        } else {
            Ok(None)
        }
    }
}

impl<Tx: DbTxMut> StateProvider for HistoricalStateProvider<Tx> {
    fn nonce(&self, address: ContractAddress) -> ProviderResult<Option<Nonce>> {
        if let res @ Some(..) = self.base_provider.nonce(address)? {
            return Ok(res);
        }

        if self.block_number > self.forked_block_number {
            return Ok(None);
        }

        if let res @ Some(nonce) = self.backend_client.get_nonce(address, self.block_number)? {
            let block = self.base_provider.block();
            let entry = ContractNonceChange { contract_address: address, nonce };

            self.base_provider.tx().put::<tables::NonceChangeHistory>(block, entry)?;
            Ok(res)
        } else {
            Ok(None)
        }
    }

    fn class_hash_of_contract(
        &self,
        address: ContractAddress,
    ) -> ProviderResult<Option<ClassHash>> {
        if let res @ Some(..) = self.base_provider.class_hash_of_contract(address)? {
            return Ok(res);
        }

        if self.block_number > self.forked_block_number {
            return Ok(None);
        }

        if let res @ Some(hash) =
            self.backend_client.get_class_hash_at(address, self.block_number)?
        {
            let block = self.base_provider.block();
            // TODO: this is technically wrong, we probably should insert the
            // `ClassChangeHistory` entry on the state update level instead.
            let entry = ContractClassChange::deployed(address, hash);

            self.base_provider.tx().put::<tables::ClassChangeHistory>(block, entry)?;
            Ok(res)
        } else {
            Ok(None)
        }
    }

    fn storage(
        &self,
        address: ContractAddress,
        key: StorageKey,
    ) -> ProviderResult<Option<StorageValue>> {
        if let res @ Some(..) = self.base_provider.storage(address, key)? {
            return Ok(res);
        }

        if self.block_number > self.forked_block_number {
            return Ok(None);
        }

        if let res @ Some(value) =
            self.backend_client.get_storage(address, key, self.block_number)?
        {
            let key = ContractStorageKey { contract_address: address, key };
            let block = self.base_provider.block();

            let block_list =
                self.base_provider.tx().get::<tables::StorageChangeSet>(key.clone())?;
            let mut block_list = block_list.unwrap_or_default();
            block_list.insert(block);

            self.base_provider.tx().put::<tables::StorageChangeSet>(key.clone(), block_list)?;
            let change_entry = ContractStorageEntry { key, value };
            self.base_provider.tx().put::<tables::StorageChangeHistory>(block, change_entry)?;

            Ok(res)
        } else {
            Ok(None)
        }
    }
}

impl<Tx: DbTxMut> StateProofProvider for HistoricalStateProvider<Tx> {
    fn class_multiproof(&self, classes: Vec<ClassHash>) -> ProviderResult<katana_trie::MultiProof> {
        self.base_provider.class_multiproof(classes)
    }

    fn contract_multiproof(
        &self,
        addresses: Vec<ContractAddress>,
    ) -> ProviderResult<katana_trie::MultiProof> {
        self.base_provider.contract_multiproof(addresses)
    }

    fn storage_multiproof(
        &self,
        address: ContractAddress,
        storage_keys: Vec<StorageKey>,
    ) -> ProviderResult<katana_trie::MultiProof> {
        self.base_provider.storage_multiproof(address, storage_keys)
    }
}

impl<Tx: DbTxMut> StateRootProvider for HistoricalStateProvider<Tx> {
    fn classes_root(&self) -> ProviderResult<Felt> {
        self.base_provider.classes_root()
    }

    fn contracts_root(&self) -> ProviderResult<Felt> {
        self.base_provider.contracts_root()
    }

    fn storage_root(&self, contract: ContractAddress) -> ProviderResult<Option<Felt>> {
        self.base_provider.storage_root(contract)
    }
}

impl<Tx: DbTxMut> StateWriter for ForkedProvider<Tx> {
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

impl<Tx: DbTxMut> ContractClassWriter for ForkedProvider<Tx> {
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
