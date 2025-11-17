use std::cmp::Ordering;
use std::sync::Arc;

use katana_db::abstraction::{Database, DbCursorMut, DbDupSortCursor, DbTx, DbTxMut};
use katana_db::models::contract::{ContractClassChange, ContractNonceChange};
use katana_db::models::storage::{ContractStorageEntry, ContractStorageKey, StorageEntry};
use katana_db::tables;
use katana_primitives::block::{BlockHashOrNumber, BlockNumber};
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
use crate::providers::db::DbProvider;
use crate::providers::fork::ForkedDb;
use crate::ProviderResult;

impl<Db> StateFactoryProvider for ForkedProvider<Db>
where
    Db: Database + 'static,
{
    fn latest(&self) -> ProviderResult<Box<dyn StateProvider>> {
        let tx = self.local_db.db().tx()?;
        let db = self.local_db.clone();
        let provider = db::state::LatestStateProvider::new(tx);
        Ok(Box::new(LatestStateProvider { db, provider, fork_db: self.fork_db.clone() }))
    }

    fn historical(
        &self,
        block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<Box<dyn StateProvider>>> {
        let block_number = match block_id {
            BlockHashOrNumber::Hash(hash) => self.local_db.block_number_by_hash(hash)?,

            BlockHashOrNumber::Num(num) => {
                let latest_num = match self.local_db.latest_number() {
                    Ok(num) => num,
                    // return the fork block number if local db return this error. this can only
                    // happen whne the ForkedProvider is constructed without
                    // inserting any locally produced blocks.
                    Err(ProviderError::MissingLatestBlockNumber) => self.block_id(),
                    Err(err) => return Err(err),
                };

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

        Ok(Some(Box::new(HistoricalStateProvider::new(db, self.fork_db.clone(), tx, block))))
    }
}

#[derive(Debug)]
struct LatestStateProvider<Db: Database> {
    db: Arc<DbProvider<Db>>,
    fork_db: Arc<ForkedDb>,
    provider: db::state::LatestStateProvider<Db::Tx>,
}

impl<Db> ContractClassProvider for LatestStateProvider<Db>
where
    Db: Database,
{
    fn class(&self, hash: ClassHash) -> ProviderResult<Option<ContractClass>> {
        if let Some(class) = self.provider.class(hash)? {
            Ok(Some(class))
        } else if let Some(class) =
            self.fork_db.backend.get_class_at(hash, self.fork_db.block_id)?
        {
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
            self.fork_db.backend.get_compiled_class_hash(hash, self.fork_db.block_id)?
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
        } else if let Some(nonce) =
            self.fork_db.backend.get_nonce(address, self.fork_db.block_id)?
        {
            let class_hash = self
                .fork_db
                .backend
                .get_class_hash_at(address, self.fork_db.block_id)?
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
            self.fork_db.backend.get_class_hash_at(address, self.fork_db.block_id)?
        {
            let nonce = self
                .fork_db
                .backend
                .get_nonce(address, self.fork_db.block_id)?
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
        } else if let Some(value) =
            self.fork_db.backend.get_storage(address, key, self.fork_db.block_id)?
        {
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
    local_db: Arc<DbProvider<Db>>,
    fork_db: Arc<ForkedDb>,
    provider: db::state::HistoricalStateProvider<Db::Tx>,
}

impl<Db: Database> HistoricalStateProvider<Db> {
    fn new(
        local_db: Arc<DbProvider<Db>>,
        fork_db: Arc<ForkedDb>,
        tx: Db::Tx,
        block: BlockNumber,
    ) -> Self {
        let provider = db::state::HistoricalStateProvider::new(tx, block);
        Self { local_db, fork_db, provider }
    }
}

impl<Db: Database> ContractClassProvider for HistoricalStateProvider<Db> {
    fn class(&self, hash: ClassHash) -> ProviderResult<Option<ContractClass>> {
        if let res @ Some(..) = self.provider.class(hash)? {
            return Ok(res);
        }

        if self.provider.block() > self.fork_db.block_id {
            return Ok(None);
        }

        if let class @ Some(..) = self.fork_db.backend.get_class_at(hash, self.provider.block())? {
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

        if self.provider.block() > self.fork_db.block_id {
            return Ok(None);
        }

        if let Some(compiled_hash) =
            self.fork_db.backend.get_compiled_class_hash(hash, self.provider.block())?
        {
            self.local_db.db().tx_mut()?.put::<tables::CompiledClassHashes>(hash, compiled_hash)?;
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

        if self.provider.block() > self.fork_db.block_id {
            return Ok(None);
        }

        if let res @ Some(nonce) = self.fork_db.backend.get_nonce(address, self.provider.block())? {
            let block = self.provider.block();
            let entry = ContractNonceChange { contract_address: address, nonce };

            self.local_db.db().tx_mut()?.put::<tables::NonceChangeHistory>(block, entry)?;
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

        if self.provider.block() > self.fork_db.block_id {
            return Ok(None);
        }

        if let res @ Some(hash) =
            self.fork_db.backend.get_class_hash_at(address, self.provider.block())?
        {
            let block = self.provider.block();
            // TODO: this is technically wrong, we probably should insert the
            // `ClassChangeHistory` entry on the state update level instead.
            let entry = ContractClassChange::deployed(address, hash);

            self.local_db.db().tx_mut()?.put::<tables::ClassChangeHistory>(block, entry)?;
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

        if self.provider.block() > self.fork_db.block_id {
            return Ok(None);
        }

        if let res @ Some(value) =
            self.fork_db.backend.get_storage(address, key, self.provider.block())?
        {
            let key = ContractStorageKey { contract_address: address, key };
            let block = self.provider.block();

            let block_list = self.provider.tx().get::<tables::StorageChangeSet>(key.clone())?;
            let mut block_list = block_list.unwrap_or_default();
            block_list.insert(block);

            self.local_db
                .db()
                .tx_mut()?
                .put::<tables::StorageChangeSet>(key.clone(), block_list)?;
            let change_entry = ContractStorageEntry { key, value };
            self.local_db
                .db()
                .tx_mut()?
                .put::<tables::StorageChangeHistory>(block, change_entry)?;

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
    fn state_root(&self) -> ProviderResult<Felt> {
        match self.provider.state_root() {
            Ok(root) => Ok(root),

            Err(ProviderError::MissingBlockHeader(..)) => {
                let block_id = BlockHashOrNumber::from(self.provider.block());

                if let Some(header) = self.fork_db.db.header(block_id)? {
                    return Ok(header.state_root);
                }

                if self.fork_db.fetch_historical_blocks(block_id)? {
                    let header = self.fork_db.db.header(block_id)?.unwrap();
                    Ok(header.state_root)
                } else {
                    Err(ProviderError::MissingBlockHeader(self.provider.block()))
                }
            }

            Err(err) => Err(err),
        }
    }

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


impl<Db: Database> ForkedProvider<Db> {
    pub fn latest_with_tx<'a>(
        &self,
        tx: &'a Db::TxMut,
    ) -> ProviderResult<MutableLatestStateProvider<'a, Db>> {
        let db = self.provider.clone();
        let backend = self.backend.clone();
        Ok(MutableLatestStateProvider { db, backend, tx })
    }
}

#[derive(Debug)]
pub struct MutableLatestStateProvider<'a, Db: Database> {
    pub db: Arc<DbProvider<Db>>,
    pub backend: BackendClient,
    pub tx: &'a Db::TxMut,
}

impl<Db> ContractClassProvider for MutableLatestStateProvider<'_, Db>
where
    Db: Database,
{
    fn class(&self, hash: ClassHash) -> ProviderResult<Option<ContractClass>> {
        if let Some(class) = self.tx.get::<tables::Classes>(hash)? {
            Ok(Some(class.into()))
        } else if let Some(class) = self.backend.get_class_at(hash)? {
            self.tx.put::<tables::Classes>(hash, class.clone().into())?;
            Ok(Some(class))
        } else {
            Ok(None)
        }
    }

    fn compiled_class_hash_of_class_hash(
        &self,
        hash: ClassHash,
    ) -> ProviderResult<Option<CompiledClassHash>> {
        if let res @ Some(..) = self.tx.get::<tables::CompiledClassHashes>(hash)? {
            Ok(res)
        } else if let Some(compiled_hash) = self.backend.get_compiled_class_hash(hash)? {
            self.tx.put::<tables::CompiledClassHashes>(hash, compiled_hash)?;
            Ok(Some(compiled_hash))
        } else {
            Ok(None)
        }
    }
}

impl<Db> StateProvider for MutableLatestStateProvider<'_, Db>
where
    Db: Database,
{
    fn nonce(&self, address: ContractAddress) -> ProviderResult<Option<Nonce>> {
        let info = self.tx.get::<tables::ContractInfo>(address)?;
        if let res @ Some(..) = info.map(|info| info.nonce) {
            Ok(res)
        } else if let Some(nonce) = self.backend.get_nonce(address)? {
            let class_hash = self
                .backend
                .get_class_hash_at(address)?
                .ok_or(ProviderError::MissingContractClassHash { address })?;

            let entry = GenericContractInfo { nonce, class_hash };
            self.tx.put::<tables::ContractInfo>(address, entry)?;
            Ok(Some(nonce))
        } else {
            Ok(None)
        }
    }

    fn class_hash_of_contract(
        &self,
        address: ContractAddress,
    ) -> ProviderResult<Option<ClassHash>> {
        let info = self.tx.get::<tables::ContractInfo>(address)?;
        if let res @ Some(..) = info.map(|info| info.class_hash) {
            Ok(res)
        } else if let Some(class_hash) = self.backend.get_class_hash_at(address)? {
            let nonce = self.backend.get_nonce(address)?.unwrap_or(Felt::ZERO);
            let entry = GenericContractInfo { class_hash, nonce };
            self.tx.put::<tables::ContractInfo>(address, entry)?;
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
        let mut cursor = self.tx.cursor_dup_mut::<tables::ContractStorage>()?;
        let entry = cursor.seek_by_key_subkey(address, key)?;
        match entry {
            Some(entry) if entry.key == key => Ok(Some(entry.value)),
            _ => {
                if let Some(value) = self.backend.get_storage(address, key)? {
                    let entry = StorageEntry { key, value };
                    cursor.upsert(address, entry)?;
                    Ok(Some(value))
                } else {
                    Ok(None)
                }
            }
        }
    }
}

impl<Db> StateProofProvider for MutableLatestStateProvider<'_, Db>
where
    Db: Database,
{
    fn class_multiproof(&self, classes: Vec<ClassHash>) -> ProviderResult<katana_trie::MultiProof> {
        let mut trie = katana_db::trie::TrieDbFactory::new(self.tx).latest().classes_trie();
        let proofs = trie.multiproof(classes);
        Ok(proofs)
    }

    fn contract_multiproof(
        &self,
        addresses: Vec<ContractAddress>,
    ) -> ProviderResult<katana_trie::MultiProof> {
        let mut trie = katana_db::trie::TrieDbFactory::new(self.tx).latest().contracts_trie();
        let proofs = trie.multiproof(addresses);
        Ok(proofs)
    }

    fn storage_multiproof(
        &self,
        address: ContractAddress,
        storage_keys: Vec<StorageKey>,
    ) -> ProviderResult<katana_trie::MultiProof> {
        let mut trie = katana_db::trie::TrieDbFactory::new(self.tx).latest().storages_trie(address);
        let proofs = trie.multiproof(storage_keys);
        Ok(proofs)
    }
}

impl<Db> StateRootProvider for MutableLatestStateProvider<'_, Db>
where
    Db: Database,
{
    fn classes_root(&self) -> ProviderResult<Felt> {
        let trie = katana_db::trie::TrieDbFactory::new(self.tx).latest().classes_trie();
        Ok(trie.root())
    }

    fn contracts_root(&self) -> ProviderResult<Felt> {
        let trie = katana_db::trie::TrieDbFactory::new(self.tx).latest().contracts_trie();
        Ok(trie.root())
    }

    fn storage_root(&self, contract: ContractAddress) -> ProviderResult<Option<Felt>> {
        let trie = katana_db::trie::TrieDbFactory::new(self.tx).latest().storages_trie(contract);
        Ok(Some(trie.root()))
    }
}

impl<Db: Database> ForkedProvider<Db> {
    pub fn latest_with_tx<'a>(
        &self,
        tx: &'a Db::TxMut,
    ) -> ProviderResult<MutableLatestStateProvider<'a, Db>> {
        let db = self.provider.clone();
        let backend = self.backend.clone();
        Ok(MutableLatestStateProvider { db, backend, tx })
    }
}

#[derive(Debug)]
pub struct MutableLatestStateProvider<'a, Db: Database> {
    pub db: Arc<DbProvider<Db>>,
    pub backend: BackendClient,
    pub tx: &'a Db::TxMut,
}

impl<Db> ContractClassProvider for MutableLatestStateProvider<'_, Db>
where
    Db: Database,
{
    fn class(&self, hash: ClassHash) -> ProviderResult<Option<ContractClass>> {
        if let Some(class) = self.tx.get::<tables::Classes>(hash)? {
            Ok(Some(class.into()))
        } else if let Some(class) = self.backend.get_class_at(hash)? {
            self.tx.put::<tables::Classes>(hash, class.clone().into())?;
            Ok(Some(class))
        } else {
            Ok(None)
        }
    }

    fn compiled_class_hash_of_class_hash(
        &self,
        hash: ClassHash,
    ) -> ProviderResult<Option<CompiledClassHash>> {
        if let res @ Some(..) = self.tx.get::<tables::CompiledClassHashes>(hash)? {
            Ok(res)
        } else if let Some(compiled_hash) = self.backend.get_compiled_class_hash(hash)? {
            self.tx.put::<tables::CompiledClassHashes>(hash, compiled_hash)?;
            Ok(Some(compiled_hash))
        } else {
            Ok(None)
        }
    }
}

impl<Db> StateProvider for MutableLatestStateProvider<'_, Db>
where
    Db: Database,
{
    fn nonce(&self, address: ContractAddress) -> ProviderResult<Option<Nonce>> {
        let info = self.tx.get::<tables::ContractInfo>(address)?;
        if let res @ Some(..) = info.map(|info| info.nonce) {
            Ok(res)
        } else if let Some(nonce) = self.backend.get_nonce(address)? {
            let class_hash = self
                .backend
                .get_class_hash_at(address)?
                .ok_or(ProviderError::MissingContractClassHash { address })?;

            let entry = GenericContractInfo { nonce, class_hash };
            self.tx.put::<tables::ContractInfo>(address, entry)?;
            Ok(Some(nonce))
        } else {
            Ok(None)
        }
    }

    fn class_hash_of_contract(
        &self,
        address: ContractAddress,
    ) -> ProviderResult<Option<ClassHash>> {
        let info = self.tx.get::<tables::ContractInfo>(address)?;
        if let res @ Some(..) = info.map(|info| info.class_hash) {
            Ok(res)
        } else if let Some(class_hash) = self.backend.get_class_hash_at(address)? {
            let nonce = self.backend.get_nonce(address)?.unwrap_or(Felt::ZERO);
            let entry = GenericContractInfo { class_hash, nonce };
            self.tx.put::<tables::ContractInfo>(address, entry)?;
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
        let mut cursor = self.tx.cursor_dup_mut::<tables::ContractStorage>()?;
        let entry = cursor.seek_by_key_subkey(address, key)?;
        match entry {
            Some(entry) if entry.key == key => Ok(Some(entry.value)),
            _ => {
                if let Some(value) = self.backend.get_storage(address, key)? {
                    let entry = StorageEntry { key, value };
                    cursor.upsert(address, entry)?;
                    Ok(Some(value))
                } else {
                    Ok(None)
                }
            }
        }
    }
}

impl<Db> StateProofProvider for MutableLatestStateProvider<'_, Db>
where
    Db: Database,
{
    fn class_multiproof(&self, classes: Vec<ClassHash>) -> ProviderResult<katana_trie::MultiProof> {
        let mut trie = katana_db::trie::TrieDbFactory::new(self.tx).latest().classes_trie();
        let proofs = trie.multiproof(classes);
        Ok(proofs)
    }

    fn contract_multiproof(
        &self,
        addresses: Vec<ContractAddress>,
    ) -> ProviderResult<katana_trie::MultiProof> {
        let mut trie = katana_db::trie::TrieDbFactory::new(self.tx).latest().contracts_trie();
        let proofs = trie.multiproof(addresses);
        Ok(proofs)
    }

    fn storage_multiproof(
        &self,
        address: ContractAddress,
        storage_keys: Vec<StorageKey>,
    ) -> ProviderResult<katana_trie::MultiProof> {
        let mut trie = katana_db::trie::TrieDbFactory::new(self.tx).latest().storages_trie(address);
        let proofs = trie.multiproof(storage_keys);
        Ok(proofs)
    }
}

impl<Db> StateRootProvider for MutableLatestStateProvider<'_, Db>
where
    Db: Database,
{
    fn classes_root(&self) -> ProviderResult<Felt> {
        let trie = katana_db::trie::TrieDbFactory::new(self.tx).latest().classes_trie();
        Ok(trie.root())
    }

    fn contracts_root(&self) -> ProviderResult<Felt> {
        let trie = katana_db::trie::TrieDbFactory::new(self.tx).latest().contracts_trie();
        Ok(trie.root())
    }

    fn storage_root(&self, contract: ContractAddress) -> ProviderResult<Option<Felt>> {
        let trie = katana_db::trie::TrieDbFactory::new(self.tx).latest().storages_trie(contract);
        Ok(Some(trie.root()))
    }
}
