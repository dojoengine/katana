use std::cmp::Ordering;
use std::sync::Arc;

use katana_db::abstraction::{Database, DbTx, DbTxMut};
use katana_db::models::contract::{ContractClassChange, ContractNonceChange};
use katana_db::models::storage::{ContractStorageEntry, ContractStorageKey, StorageEntry};
use katana_db::tables;
use katana_fork::Backend;
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
use crate::providers::db::DbProvider;
use crate::ProviderResult;

impl<Db> StateFactoryProvider for ForkedProvider<Db>
where
    Db: Database + 'static,
{
    fn latest(&self) -> ProviderResult<Box<dyn StateProvider>> {
        let tx = self.local_db.db().tx()?;
        let db = self.local_db.clone();
        let provider = db::state::LatestStateProvider::new(tx);
        Ok(Box::new(LatestStateProvider {
            db,
            backend: self.backend.clone(),
            provider,
            forked_block_id: self.block_id,
        }))
    }

    fn historical(
        &self,
        block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<Box<dyn StateProvider>>> {
        let block_number = match block_id {
            BlockHashOrNumber::Hash(hash) => self.local_db.block_number_by_hash(hash)?,

            BlockHashOrNumber::Num(num) => {
                let latest_num = self.local_db.latest_number()?;

                match num.cmp(&latest_num) {
                    Ordering::Less => Some(num),
                    Ordering::Greater => return Ok(None),
                    Ordering::Equal => return self.latest().map(Some),
                }
            }
        };

        let Some(block) = block_number else { return Ok(None) };

        let db = self.local_db.clone();
        let tx = db.db().tx()?;
        let client = self.backend.clone();

        Ok(Some(Box::new(HistoricalStateProvider::new(
            db,
            tx,
            block,
            client,
            block,
            self.block_id,
        ))))
    }
}

#[derive(Debug)]
struct LatestStateProvider<Db: Database> {
    db: Arc<DbProvider<Db>>,
    backend: Backend,
    provider: db::state::LatestStateProvider<Db::Tx>,
    forked_block_id: BlockNumber,
}

impl<Db> ContractClassProvider for LatestStateProvider<Db>
where
    Db: Database,
{
    fn class(&self, hash: ClassHash) -> ProviderResult<Option<ContractClass>> {
        if let Some(class) = self.provider.class(hash)? {
            Ok(Some(class))
        } else if let Some(class) = self.backend.get_class_at(hash, self.forked_block_id)? {
            self.db.db().update(|tx| tx.put::<tables::Classes>(hash, class.clone().into()))??;
            Ok(Some(class))
        } else {
            Ok(None)
        }
    }

    fn compiled_class_hash_of_class_hash(
        &self,
        hash: ClassHash,
    ) -> ProviderResult<Option<CompiledClassHash>> {
        if let res @ Some(..) = self.provider.compiled_class_hash_of_class_hash(hash)? {
            Ok(res)
        } else if let Some(compiled_hash) =
            self.backend.get_compiled_class_hash(hash, self.forked_block_id)?
        {
            self.db
                .db()
                .update(|tx| tx.put::<tables::CompiledClassHashes>(hash, compiled_hash))??;
            Ok(Some(compiled_hash))
        } else {
            Ok(None)
        }
    }
}

impl<Db> StateProvider for LatestStateProvider<Db>
where
    Db: Database,
{
    fn nonce(&self, address: ContractAddress) -> ProviderResult<Option<Nonce>> {
        if let res @ Some(..) = self.provider.nonce(address)? {
            Ok(res)
        } else if let Some(nonce) = self.backend.get_nonce(address, self.forked_block_id)? {
            let class_hash = self
                .backend
                .get_class_hash_at(address, self.forked_block_id)?
                .ok_or(ProviderError::MissingContractClassHash { address })?;

            let entry = GenericContractInfo { nonce, class_hash };
            self.db.db().update(|tx| tx.put::<tables::ContractInfo>(address, entry))??;

            Ok(Some(nonce))
        } else {
            Ok(None)
        }
    }

    fn class_hash_of_contract(
        &self,
        address: ContractAddress,
    ) -> ProviderResult<Option<ClassHash>> {
        if let res @ Some(..) = self.provider.class_hash_of_contract(address)? {
            Ok(res)
        } else if let Some(class_hash) =
            self.backend.get_class_hash_at(address, self.forked_block_id)?
        {
            let nonce = self
                .backend
                .get_nonce(address, self.forked_block_id)?
                .ok_or(ProviderError::MissingContractNonce { address })?;

            let entry = GenericContractInfo { class_hash, nonce };
            self.db.db().update(|tx| tx.put::<tables::ContractInfo>(address, entry))??;

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
        if let res @ Some(..) = self.provider.storage(address, key)? {
            Ok(res)
        } else if let Some(value) = self.backend.get_storage(address, key, self.forked_block_id)? {
            let entry = StorageEntry { key, value };
            self.db.db().update(|tx| tx.put::<tables::ContractStorage>(address, entry))??;
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }
}

impl<Db> StateProofProvider for LatestStateProvider<Db>
where
    Db: Database,
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

impl<Db> StateRootProvider for LatestStateProvider<Db>
where
    Db: Database,
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
struct HistoricalStateProvider<Db: Database> {
    db: Arc<DbProvider<Db>>,
    backend: Backend,
    provider: db::state::HistoricalStateProvider<Db::Tx>,
    block_number: BlockNumber,
    forked_block_number: BlockNumber,
}

impl<Db: Database> HistoricalStateProvider<Db> {
    fn new(
        db: Arc<DbProvider<Db>>,
        tx: Db::Tx,
        block: BlockNumber,
        backend: Backend,
        block_number: BlockNumber,
        forked_block_number: BlockNumber,
    ) -> Self {
        let provider = db::state::HistoricalStateProvider::new(tx, block);
        Self { db, backend, provider, block_number, forked_block_number }
    }
}

impl<Db: Database> ContractClassProvider for HistoricalStateProvider<Db> {
    fn class(&self, hash: ClassHash) -> ProviderResult<Option<ContractClass>> {
        if let res @ Some(..) = self.provider.class(hash)? {
            return Ok(res);
        }

        if self.block_number > self.forked_block_number {
            return Ok(None);
        }

        if let class @ Some(..) = self.backend.get_class_at(hash, self.block_number)? {
            Ok(class)
        } else {
            Ok(None)
        }
    }

    fn compiled_class_hash_of_class_hash(
        &self,
        hash: ClassHash,
    ) -> ProviderResult<Option<CompiledClassHash>> {
        if let res @ Some(..) = self.provider.compiled_class_hash_of_class_hash(hash)? {
            return Ok(res);
        }

        if self.block_number > self.forked_block_number {
            return Ok(None);
        }

        if let Some(compiled_hash) =
            self.backend.get_compiled_class_hash(hash, self.block_number)?
        {
            self.db.db().tx_mut()?.put::<tables::CompiledClassHashes>(hash, compiled_hash)?;
            Ok(Some(compiled_hash))
        } else {
            Ok(None)
        }
    }
}

impl<Db: Database> StateProvider for HistoricalStateProvider<Db> {
    fn nonce(&self, address: ContractAddress) -> ProviderResult<Option<Nonce>> {
        if let res @ Some(..) = self.provider.nonce(address)? {
            return Ok(res);
        }

        if self.block_number > self.forked_block_number {
            return Ok(None);
        }

        if let res @ Some(nonce) = self.backend.get_nonce(address, self.block_number)? {
            let block = self.provider.block();
            let entry = ContractNonceChange { contract_address: address, nonce };

            self.db.db().tx_mut()?.put::<tables::NonceChangeHistory>(block, entry)?;
            Ok(res)
        } else {
            Ok(None)
        }
    }

    fn class_hash_of_contract(
        &self,
        address: ContractAddress,
    ) -> ProviderResult<Option<ClassHash>> {
        if let res @ Some(..) = self.provider.class_hash_of_contract(address)? {
            return Ok(res);
        }

        if self.block_number > self.forked_block_number {
            return Ok(None);
        }

        if let res @ Some(hash) = self.backend.get_class_hash_at(address, self.block_number)? {
            let block = self.provider.block();
            // TODO: this is technically wrong, we probably should insert the
            // `ClassChangeHistory` entry on the state update level instead.
            let entry = ContractClassChange::deployed(address, hash);

            self.db.db().tx_mut()?.put::<tables::ClassChangeHistory>(block, entry)?;
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
        if let res @ Some(..) = self.provider.storage(address, key)? {
            return Ok(res);
        }

        if self.block_number > self.forked_block_number {
            return Ok(None);
        }

        if let res @ Some(value) = self.backend.get_storage(address, key, self.block_number)? {
            let key = ContractStorageKey { contract_address: address, key };
            let block = self.provider.block();

            let block_list = self.provider.tx().get::<tables::StorageChangeSet>(key.clone())?;
            let mut block_list = block_list.unwrap_or_default();
            block_list.insert(block);

            self.db.db().tx_mut()?.put::<tables::StorageChangeSet>(key.clone(), block_list)?;
            let change_entry = ContractStorageEntry { key, value };
            self.db.db().tx_mut()?.put::<tables::StorageChangeHistory>(block, change_entry)?;

            Ok(res)
        } else {
            Ok(None)
        }
    }
}

impl<Db: Database> StateProofProvider for HistoricalStateProvider<Db> {
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

impl<Db: Database> StateRootProvider for HistoricalStateProvider<Db> {
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
        self.local_db.set_class_hash_of_contract(address, class_hash)
    }

    fn set_nonce(&self, address: ContractAddress, nonce: Nonce) -> ProviderResult<()> {
        self.local_db.set_nonce(address, nonce)
    }

    fn set_storage(
        &self,
        address: ContractAddress,
        storage_key: StorageKey,
        storage_value: StorageValue,
    ) -> ProviderResult<()> {
        self.local_db.set_storage(address, storage_key, storage_value)
    }
}

impl<Db: Database> ContractClassWriter for ForkedProvider<Db> {
    fn set_class(&self, hash: ClassHash, class: ContractClass) -> ProviderResult<()> {
        self.local_db.set_class(hash, class)
    }

    fn set_compiled_class_hash_of_class_hash(
        &self,
        hash: ClassHash,
        compiled_hash: CompiledClassHash,
    ) -> ProviderResult<()> {
        self.local_db.set_compiled_class_hash_of_class_hash(hash, compiled_hash)
    }
}
