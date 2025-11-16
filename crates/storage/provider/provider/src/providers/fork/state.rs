use std::cmp::Ordering;

use katana_db::abstraction::{DbTx, DbTxMut};
use katana_db::models::contract::{ContractClassChange, ContractNonceChange};
use katana_db::models::storage::{ContractStorageEntry, ContractStorageKey, StorageEntry};
use katana_db::tables;
use katana_primitives::block::BlockHashOrNumber;
use katana_primitives::class::{ClassHash, CompiledClassHash, ContractClass};
use katana_primitives::contract::{GenericContractInfo, Nonce, StorageKey, StorageValue};
use katana_primitives::{ContractAddress, Felt};
use katana_provider_api::block::{BlockNumberProvider, HeaderProvider};
use katana_provider_api::contract::{ContractClassProvider, ContractClassWriter};
use katana_provider_api::state::{
    StateFactoryProvider, StateProofProvider, StateProvider, StateRootProvider, StateWriter,
};
use katana_provider_api::ProviderError;

use super::db::{self};
use super::ForkedProvider;
use crate::providers::fork::ForkedDb;
use crate::ProviderResult;

impl<Tx1: DbTx, Tx2: DbTxMut> StateFactoryProvider for ForkedProvider<Tx1, Tx2> {
    fn latest(&self) -> ProviderResult<Box<dyn StateProvider>> {
        let local_provider = db::state::LatestStateProvider(self.local_db.tx().clone());
        let fork_provider = self.fork_db.clone();
        Ok(Box::new(LatestStateProvider { local_provider, fork_provider }))
    }

    fn historical(
        &self,
        block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<Box<dyn StateProvider>>> {
        let block_number = match block_id {
            BlockHashOrNumber::Num(num) => {
                let latest_num = self.local_db.latest_number()?;

                match num.cmp(&latest_num) {
                    Ordering::Less => Some(num),
                    Ordering::Equal => return self.latest().map(Some),
                    Ordering::Greater => return Ok(None),
                }
            }

            BlockHashOrNumber::Hash(hash) => {
                if let num @ Some(..) = self.local_db.block_number_by_hash(hash)? {
                    num
                } else if let num @ Some(..) = self.fork_db.db.block_number_by_hash(hash)? {
                    num
                } else {
                    None
                }
            }
        };

        let Some(block) = block_number else { return Ok(None) };

        let local_provider =
            db::state::HistoricalStateProvider::new(self.local_db.tx().clone(), block);

        Ok(Some(Box::new(HistoricalStateProvider {
            local_provider,
            fork_provider: self.fork_db.clone(),
        })))
    }
}

#[derive(Debug)]
struct LatestStateProvider<Tx1: DbTx, Tx2: DbTxMut> {
    local_provider: db::state::LatestStateProvider<Tx1>,
    fork_provider: ForkedDb<Tx2>,
}

impl<Tx1: DbTx, Tx2: DbTxMut> ContractClassProvider for LatestStateProvider<Tx1, Tx2> {
    fn class(&self, hash: ClassHash) -> ProviderResult<Option<ContractClass>> {
        if let Some(class) = self.local_provider.class(hash)? {
            Ok(Some(class))
        } else if let Some(class) =
            self.fork_provider.backend.get_class_at(hash, self.fork_provider.block_id)?
        {
            self.fork_provider.db.tx().put::<tables::Classes>(hash, class.clone().into())?;
            Ok(Some(class))
        } else {
            Ok(None)
        }
    }

    fn compiled_class_hash_of_class_hash(
        &self,
        hash: ClassHash,
    ) -> ProviderResult<Option<CompiledClassHash>> {
        if let res @ Some(..) = self.local_provider.compiled_class_hash_of_class_hash(hash)? {
            Ok(res)
        } else if let Some(compiled_hash) =
            self.fork_provider.backend.get_compiled_class_hash(hash, self.fork_provider.block_id)?
        {
            self.fork_provider.db.tx().put::<tables::CompiledClassHashes>(hash, compiled_hash)?;
            Ok(Some(compiled_hash))
        } else {
            Ok(None)
        }
    }
}

impl<Tx1: DbTx, Tx2: DbTxMut> StateProvider for LatestStateProvider<Tx1, Tx2> {
    fn nonce(&self, address: ContractAddress) -> ProviderResult<Option<Nonce>> {
        if let res @ Some(..) = self.local_provider.nonce(address)? {
            Ok(res)
        } else if let Some(nonce) =
            self.fork_provider.backend.get_nonce(address, self.fork_provider.block_id)?
        {
            let class_hash = self
                .fork_provider
                .backend
                .get_class_hash_at(address, self.fork_provider.block_id)?
                .ok_or(ProviderError::MissingContractClassHash { address })?;

            let entry = GenericContractInfo { nonce, class_hash };
            self.fork_provider.db.tx().put::<tables::ContractInfo>(address, entry)?;

            Ok(Some(nonce))
        } else {
            Ok(None)
        }
    }

    fn class_hash_of_contract(
        &self,
        address: ContractAddress,
    ) -> ProviderResult<Option<ClassHash>> {
        if let res @ Some(..) = self.local_provider.class_hash_of_contract(address)? {
            Ok(res)
        } else if let Some(class_hash) =
            self.fork_provider.backend.get_class_hash_at(address, self.fork_provider.block_id)?
        {
            let nonce = self
                .fork_provider
                .backend
                .get_nonce(address, self.fork_provider.block_id)?
                .ok_or(ProviderError::MissingContractNonce { address })?;

            let entry = GenericContractInfo { class_hash, nonce };
            self.fork_provider.db.tx().put::<tables::ContractInfo>(address, entry)?;

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
        if let res @ Some(..) = self.local_provider.storage(address, key)? {
            Ok(res)
        } else if let Some(value) =
            self.fork_provider.backend.get_storage(address, key, self.fork_provider.block_id)?
        {
            let entry = StorageEntry { key, value };
            self.fork_provider.db.tx().put::<tables::ContractStorage>(address, entry)?;
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }
}

impl<Tx1: DbTx, Tx2: DbTxMut> StateProofProvider for LatestStateProvider<Tx1, Tx2> {
    fn class_multiproof(&self, classes: Vec<ClassHash>) -> ProviderResult<katana_trie::MultiProof> {
        self.local_provider.class_multiproof(classes)
    }

    fn contract_multiproof(
        &self,
        addresses: Vec<ContractAddress>,
    ) -> ProviderResult<katana_trie::MultiProof> {
        self.local_provider.contract_multiproof(addresses)
    }

    fn storage_multiproof(
        &self,
        address: ContractAddress,
        storage_keys: Vec<StorageKey>,
    ) -> ProviderResult<katana_trie::MultiProof> {
        self.local_provider.storage_multiproof(address, storage_keys)
    }
}

impl<Tx1: DbTx, Tx2: DbTxMut> StateRootProvider for LatestStateProvider<Tx1, Tx2> {
    fn classes_root(&self) -> ProviderResult<Felt> {
        self.local_provider.classes_root()
    }

    fn contracts_root(&self) -> ProviderResult<Felt> {
        self.local_provider.contracts_root()
    }

    fn storage_root(&self, contract: ContractAddress) -> ProviderResult<Option<Felt>> {
        self.local_provider.storage_root(contract)
    }
}

#[derive(Debug)]
struct HistoricalStateProvider<Tx1: DbTx, Tx2: DbTxMut> {
    local_provider: db::state::HistoricalStateProvider<Tx1>,
    fork_provider: ForkedDb<Tx2>,
}

impl<Tx1: DbTx, Tx2: DbTxMut> ContractClassProvider for HistoricalStateProvider<Tx1, Tx2> {
    fn class(&self, hash: ClassHash) -> ProviderResult<Option<ContractClass>> {
        if let res @ Some(..) = self.local_provider.class(hash)? {
            return Ok(res);
        }

        if self.local_provider.block() > self.fork_provider.block_id {
            return Ok(None);
        }

        if let class @ Some(..) =
            self.fork_provider.backend.get_class_at(hash, self.local_provider.block())?
        {
            Ok(class)
        } else {
            Ok(None)
        }
    }

    fn compiled_class_hash_of_class_hash(
        &self,
        hash: ClassHash,
    ) -> ProviderResult<Option<CompiledClassHash>> {
        if let res @ Some(..) = self.local_provider.compiled_class_hash_of_class_hash(hash)? {
            return Ok(res);
        }

        if self.local_provider.block() > self.fork_provider.block_id {
            return Ok(None);
        }

        if let Some(compiled_hash) =
            self.fork_provider.backend.get_compiled_class_hash(hash, self.local_provider.block())?
        {
            self.fork_provider.db.tx().put::<tables::CompiledClassHashes>(hash, compiled_hash)?;
            Ok(Some(compiled_hash))
        } else {
            Ok(None)
        }
    }
}

impl<Tx1: DbTx, Tx2: DbTxMut> StateProvider for HistoricalStateProvider<Tx1, Tx2> {
    fn nonce(&self, address: ContractAddress) -> ProviderResult<Option<Nonce>> {
        if let res @ Some(..) = self.local_provider.nonce(address)? {
            return Ok(res);
        }

        if self.local_provider.block() > self.fork_provider.block_id {
            return Ok(None);
        }

        if let res @ Some(nonce) =
            self.fork_provider.backend.get_nonce(address, self.local_provider.block())?
        {
            let block = self.local_provider.block();
            let entry = ContractNonceChange { contract_address: address, nonce };

            self.fork_provider.db.tx().put::<tables::NonceChangeHistory>(block, entry)?;
            Ok(res)
        } else {
            Ok(None)
        }
    }

    fn class_hash_of_contract(
        &self,
        address: ContractAddress,
    ) -> ProviderResult<Option<ClassHash>> {
        if let res @ Some(..) = self.local_provider.class_hash_of_contract(address)? {
            return Ok(res);
        }

        if self.local_provider.block() > self.fork_provider.block_id {
            return Ok(None);
        }

        if let res @ Some(hash) =
            self.fork_provider.backend.get_class_hash_at(address, self.local_provider.block())?
        {
            let block = self.local_provider.block();
            // TODO: this is technically wrong, we probably should insert the
            // `ClassChangeHistory` entry on the state update level instead.
            let entry = ContractClassChange::deployed(address, hash);

            self.fork_provider.db.tx().put::<tables::ClassChangeHistory>(block, entry)?;
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
        if let res @ Some(..) = self.local_provider.storage(address, key)? {
            return Ok(res);
        }

        if self.local_provider.block() > self.fork_provider.block_id {
            return Ok(None);
        }

        if let res @ Some(value) =
            self.fork_provider.backend.get_storage(address, key, self.local_provider.block())?
        {
            let key = ContractStorageKey { contract_address: address, key };
            let block = self.local_provider.block();

            let block_list =
                self.local_provider.tx().get::<tables::StorageChangeSet>(key.clone())?;
            let mut block_list = block_list.unwrap_or_default();
            block_list.insert(block);

            self.fork_provider.db.tx().put::<tables::StorageChangeSet>(key.clone(), block_list)?;
            let change_entry = ContractStorageEntry { key, value };
            self.fork_provider.db.tx().put::<tables::StorageChangeHistory>(block, change_entry)?;

            Ok(res)
        } else {
            Ok(None)
        }
    }
}

impl<Tx1: DbTx, Tx2: DbTxMut> StateProofProvider for HistoricalStateProvider<Tx1, Tx2> {
    fn class_multiproof(&self, classes: Vec<ClassHash>) -> ProviderResult<katana_trie::MultiProof> {
        self.local_provider.class_multiproof(classes)
    }

    fn contract_multiproof(
        &self,
        addresses: Vec<ContractAddress>,
    ) -> ProviderResult<katana_trie::MultiProof> {
        self.local_provider.contract_multiproof(addresses)
    }

    fn storage_multiproof(
        &self,
        address: ContractAddress,
        storage_keys: Vec<StorageKey>,
    ) -> ProviderResult<katana_trie::MultiProof> {
        self.local_provider.storage_multiproof(address, storage_keys)
    }
}

impl<Tx1: DbTx, Tx2: DbTxMut> StateRootProvider for HistoricalStateProvider<Tx1, Tx2> {
    fn state_root(&self) -> ProviderResult<Felt> {
        match self.local_provider.state_root() {
            Ok(root) => Ok(root),

            Err(ProviderError::MissingBlockHeader(..)) => {
                let block_id = BlockHashOrNumber::from(self.local_provider.block());

                if let Some(header) = self.fork_provider.db.header(block_id)? {
                    return Ok(header.state_root);
                }

                if self.fork_provider.fetch_historical_blocks(block_id)? {
                    let header = self.fork_provider.db.header(block_id)?.unwrap();
                    Ok(header.state_root)
                } else {
                    Err(ProviderError::MissingBlockHeader(self.local_provider.block()))
                }
            }

            Err(err) => Err(err),
        }
    }

    fn classes_root(&self) -> ProviderResult<Felt> {
        self.local_provider.classes_root()
    }

    fn contracts_root(&self) -> ProviderResult<Felt> {
        self.local_provider.contracts_root()
    }

    fn storage_root(&self, contract: ContractAddress) -> ProviderResult<Option<Felt>> {
        self.local_provider.storage_root(contract)
    }
}

impl<Tx1: DbTxMut, Tx2: DbTxMut> StateWriter for ForkedProvider<Tx1, Tx2> {
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

impl<Tx1: DbTxMut, Tx2: DbTxMut> ContractClassWriter for ForkedProvider<Tx1, Tx2> {
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
