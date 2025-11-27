use std::cmp::Ordering;

use katana_db::abstraction::{DbTx, DbTxMut};
use katana_db::models::contract::{ContractClassChange, ContractNonceChange};
use katana_db::models::storage::{ContractStorageEntry, ContractStorageKey, StorageEntry};
use katana_db::tables;
use katana_db::trie::TrieDbFactory;
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
use katana_rpc_types::ContractStorageKeys;

use super::db::{self};
use super::ForkedProvider;
use crate::providers::fork::ForkedDb;
use crate::{MutableProvider, ProviderFactory, ProviderResult};

impl<Tx1: DbTx> StateFactoryProvider for ForkedProvider<Tx1> {
    fn latest(&self) -> ProviderResult<Box<dyn StateProvider>> {
        let local_provider = db::state::LatestStateProvider(self.local_db.clone());
        let fork_provider = self.fork_db.clone();
        Ok(Box::new(LatestStateProvider { local_provider, fork_provider }))
    }

    fn historical(
        &self,
        block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<Box<dyn StateProvider>>> {
        let block_number = match block_id {
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
                    Ordering::Equal => return self.latest().map(Some),
                    Ordering::Greater => return Ok(None),
                }
            }

            BlockHashOrNumber::Hash(hash) => {
                if let num @ Some(..) = self.local_db.block_number_by_hash(hash)? {
                    num
                } else if let num @ Some(..) =
                    self.fork_db.db.provider().block_number_by_hash(hash)?
                {
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
struct LatestStateProvider<Tx1: DbTx> {
    local_provider: db::state::LatestStateProvider<Tx1>,
    fork_provider: ForkedDb,
}

impl<Tx1: DbTx> ContractClassProvider for LatestStateProvider<Tx1> {
    fn class(&self, hash: ClassHash) -> ProviderResult<Option<ContractClass>> {
        if let Some(class) = self.local_provider.class(hash)? {
            Ok(Some(class))
        } else if let Some(class) =
            self.fork_provider.backend.get_class_at(hash, self.fork_provider.block_id)?
        {
            let provider_mut = self.fork_provider.db.provider_mut();
            provider_mut.tx().put::<tables::Classes>(hash, class.clone().into())?;
            provider_mut.commit()?;

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
            let provider_mut = self.fork_provider.db.provider_mut();
            provider_mut.tx().put::<tables::CompiledClassHashes>(hash, compiled_hash)?;
            provider_mut.commit()?;

            Ok(Some(compiled_hash))
        } else {
            Ok(None)
        }
    }
}

impl<Tx1: DbTx> StateProvider for LatestStateProvider<Tx1> {
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

            let provider_mut = self.fork_provider.db.provider_mut();
            provider_mut.tx().put::<tables::ContractInfo>(address, entry)?;
            provider_mut.commit()?;

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
        let mut cursor = self.read_tx.cursor_dup::<tables::ContractStorage>()?;
        match cursor.seek_by_key_subkey(address, key)? {
            Some(entry) if entry.key == key => Ok(Some(entry.value)),
            _ => {
                if let Some(value) =
                    self.fork_db.backend.get_storage(address, key, self.fork_db.block_id)?
                {
                    let entry = StorageEntry { key, value };
                    let mut write_cursor = self.tx.cursor_dup_mut::<tables::ContractStorage>()?;
                    write_cursor.upsert(address, entry)?;
                    Ok(Some(value))
                } else {
                    Ok(None)
                }
            }
        }
    }
}

impl<'a, Db> StateProofProvider for MutableLatestStateProvider<'a, Db>
where
    Db: Database,
{
    fn class_multiproof(&self, classes: Vec<ClassHash>) -> ProviderResult<katana_trie::MultiProof> {
        let mut trie = TrieDbFactory::new(&self.read_tx).latest().classes_trie();
        Ok(trie.multiproof(classes))
    }

    fn contract_multiproof(
        &self,
        addresses: Vec<ContractAddress>,
    ) -> ProviderResult<katana_trie::MultiProof> {
        let mut trie = TrieDbFactory::new(&self.read_tx).latest().contracts_trie();
        Ok(trie.multiproof(addresses))
    }

    fn storage_multiproof(
        &self,
        address: ContractAddress,
        storage_keys: Vec<StorageKey>,
    ) -> ProviderResult<katana_trie::MultiProof> {
        let mut trie = TrieDbFactory::new(&self.read_tx).latest().storages_trie(address);
        Ok(trie.multiproof(storage_keys))
    }
}

impl<'a, Db> StateRootProvider for MutableLatestStateProvider<'a, Db>
where
    Db: Database,
{
    fn classes_root(&self) -> ProviderResult<Felt> {
        let trie = TrieDbFactory::new(&self.read_tx).latest().classes_trie();
        Ok(trie.root())
    }

    fn contracts_root(&self) -> ProviderResult<Felt> {
        let trie = TrieDbFactory::new(&self.read_tx).latest().contracts_trie();
        Ok(trie.root())
    }

    fn storage_root(&self, contract: ContractAddress) -> ProviderResult<Option<Felt>> {
        // Use tx instead of read_tx to see current changes in the trie
        let trie = TrieDbFactory::new(self.tx).latest().storages_trie(contract);
        Ok(Some(trie.root()))
    }
}

impl<Tx1: DbTx> ContractClassProvider for LatestStateProvider<Tx1> {
    fn class(&self, hash: ClassHash) -> ProviderResult<Option<ContractClass>> {
        if let Some(class) = self.local_provider.class(hash)? {
            Ok(Some(class))
        } else if let Some(class) =
            self.fork_provider.backend.get_class_at(hash, self.fork_provider.block_id)?
        {
            let provider_mut = self.fork_provider.db.provider_mut();
            provider_mut.tx().put::<tables::Classes>(hash, class.clone().into())?;
            provider_mut.commit()?;

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
            let provider_mut = self.fork_provider.db.provider_mut();
            provider_mut.tx().put::<tables::CompiledClassHashes>(hash, compiled_hash)?;
            provider_mut.commit()?;

            Ok(Some(compiled_hash))
        } else {
            Ok(None)
        }
    }
}

impl<Tx1: DbTx> StateProvider for LatestStateProvider<Tx1> {
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

            let provider_mut = self.fork_provider.db.provider_mut();
            provider_mut.tx().put::<tables::ContractInfo>(address, entry)?;
            provider_mut.commit()?;

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

            let provider_mut = self.fork_provider.db.provider_mut();
            provider_mut.tx().put::<tables::ContractInfo>(address, entry)?;
            provider_mut.commit()?;

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

            let provider_mut = self.fork_provider.db.provider_mut();
            provider_mut.tx().put::<tables::ContractStorage>(address, entry)?;
            provider_mut.commit()?;

            Ok(Some(value))
        } else {
            Ok(None)
        }
    }
}

impl<Tx1: DbTx> StateProofProvider for LatestStateProvider<Tx1> {
    fn class_multiproof(&self, classes: Vec<ClassHash>) -> ProviderResult<katana_trie::MultiProof> {
        let fork_point = self.fork_provider.block_id;
        let latest_block_number = match self.local_provider.0.latest_number() {
            Ok(num) => num,
            // return the fork block number if local db return this error. this can only happen whne
            // the ForkedProvider is constructed without inserting any locally produced
            // blocks.
            Err(ProviderError::MissingLatestBlockNumber) => self.fork_provider.block_id,
            Err(err) => return Err(err),
        };

        if latest_block_number == fork_point {
            let result = self.fork_provider.backend.get_classes_proofs(classes, fork_point)?;
            let proofs = result.expect("proofs should exist for block");

            Ok(proofs.classes_proof.nodes.into())
        } else {
            let mut tx = self.db.db().tx()?;
            let mut trie = TrieDbFactory::new(&tx).latest().classes_trie();
            let proofs = trie.multiproof(classes);
            tx.commit()?;
            Ok(proofs)
        }
    }

    fn contract_multiproof(
        &self,
        addresses: Vec<ContractAddress>,
    ) -> ProviderResult<katana_trie::MultiProof> {
        let fork_point = self.fork_provider.block_id;
        let latest_block_number = match self.local_provider.0.latest_number() {
            Ok(num) => num,
            // return the fork block number if local db return this error. this can only happen whne
            // the ForkedProvider is constructed without inserting any locally produced
            // blocks.
            Err(ProviderError::MissingLatestBlockNumber) => self.fork_provider.block_id,
            Err(err) => return Err(err),
        };

        if latest_block_number == fork_point {
            let result = self.fork_provider.backend.get_contracts_proofs(addresses, fork_point)?;
            let proofs = result.expect("proofs should exist for block");

            Ok(proofs.contracts_proof.nodes.into())
        } else {
            let mut tx = self.db.db().tx()?;
            let mut trie = TrieDbFactory::new(&tx).latest().contracts_trie();
            let proofs = trie.multiproof(addresses);
            tx.commit()?;
            Ok(proofs)
        }
    }

    fn storage_multiproof(
        &self,
        address: ContractAddress,
        storage_keys: Vec<StorageKey>,
    ) -> ProviderResult<katana_trie::MultiProof> {
        let fork_point = self.fork_provider.block_id;
        let latest_block_number = match self.local_provider.0.latest_number() {
            Ok(num) => num,
            // return the fork block number if local db return this error. this can only happen whne
            // the ForkedProvider is constructed without inserting any locally produced
            // blocks.
            Err(ProviderError::MissingLatestBlockNumber) => self.fork_provider.block_id,
            Err(err) => return Err(err),
        };

        if latest_block_number == fork_point {
            let key = vec![ContractStorageKeys { address, keys: storage_keys }];
            let result = self.fork_provider.backend.get_storages_proofs(key, fork_point)?;

            let mut proofs = result.expect("proofs should exist for block");
            let proofs = proofs.contracts_storage_proofs.nodes.pop().unwrap();

            Ok(proofs.into())
        } else {
            let mut tx = self.db.db().tx()?;
            let mut trie = TrieDbFactory::new(&tx).latest().storages_trie(address);
            let proofs = trie.multiproof(storage_keys);
            tx.commit()?;
            Ok(proofs)
        }
    }
}

impl<Tx1: DbTx> StateRootProvider for LatestStateProvider<Tx1> {
    fn classes_root(&self) -> ProviderResult<Felt> {
        let fork_point = self.fork_provider.block_id;
        let latest_block_number = match self.local_provider.0.latest_number() {
            Ok(num) => num,
            // return the fork block number if local db return this error. this can only happen whne
            // the ForkedProvider is constructed without inserting any locally produced
            // blocks.
            Err(ProviderError::MissingLatestBlockNumber) => self.fork_provider.block_id,
            Err(err) => return Err(err),
        };

        if latest_block_number == fork_point {
            let result = self.fork_provider.backend.get_global_roots(fork_point)?;
            let roots = result.expect("proofs should exist for block");

            Ok(roots.global_roots.classes_tree_root)
        } else {
            let mut tx = self.db.db().tx()?;
            let trie = TrieDbFactory::new(&tx).latest().classes_trie();
            let root = trie.root();
            tx.commit()?;
            Ok(root)
        }
    }

    fn contracts_root(&self) -> ProviderResult<Felt> {
        let fork_point = self.fork_provider.block_id;
        let latest_block_number = match self.local_provider.0.latest_number() {
            Ok(num) => num,
            // return the fork block number if local db return this error. this can only happen whne
            // the ForkedProvider is constructed without inserting any locally produced
            // blocks.
            Err(ProviderError::MissingLatestBlockNumber) => self.fork_provider.block_id,
            Err(err) => return Err(err),
        };

        if latest_block_number == fork_point {
            let result = self.fork_provider.backend.get_global_roots(fork_point)?;
            let roots = result.expect("proofs should exist for block");

            Ok(roots.global_roots.contracts_tree_root)
        } else {
            let mut tx = self.db.db().tx()?;
            let trie = TrieDbFactory::new(&tx).latest().contracts_trie();
            let root = trie.root();
            tx.commit()?;
            Ok(root)
        }
    }

    fn storage_root(&self, contract: ContractAddress) -> ProviderResult<Option<Felt>> {
        let fork_point = self.fork_provider.block_id;
        let latest_block_number = match self.local_provider.0.latest_number() {
            Ok(num) => num,
            // return the fork block number if local db return this error. this can only happen whne
            // the ForkedProvider is constructed without inserting any locally produced
            // blocks.
            Err(ProviderError::MissingLatestBlockNumber) => self.fork_provider.block_id,
            Err(err) => return Err(err),
        };

        if latest_block_number == fork_point {
            let result = self.fork_provider.backend.get_storage_root(contract, fork_point)?;
            let root = result.expect("proofs should exist for block");
            Ok(Some(root))
        } else {
            let mut tx = self.db.db().tx()?;
            let trie = TrieDbFactory::new(&tx).latest().storages_trie(contract);
            let root = trie.root();
            tx.commit()?;
            Ok(Some(root))
        }
    }
}

#[derive(Debug)]
struct HistoricalStateProvider<Tx1: DbTx> {
    local_provider: db::state::HistoricalStateProvider<Tx1>,
    fork_provider: ForkedDb,
}

impl<Tx1: DbTx> ContractClassProvider for HistoricalStateProvider<Tx1> {
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
            let provider_mut = self.fork_provider.db.provider_mut();
            provider_mut.tx().put::<tables::CompiledClassHashes>(hash, compiled_hash)?;
            provider_mut.commit()?;

            Ok(Some(compiled_hash))
        } else {
            Ok(None)
        }
    }
}

impl<Tx1: DbTx> StateProvider for HistoricalStateProvider<Tx1> {
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

            let provider_mut = self.fork_provider.db.provider_mut();
            provider_mut.tx().put::<tables::NonceChangeHistory>(block, entry)?;
            provider_mut.commit()?;

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

            let provider_mut = self.fork_provider.db.provider_mut();
            provider_mut.tx().put::<tables::ClassChangeHistory>(block, entry)?;
            provider_mut.commit()?;

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

            let change_entry = ContractStorageEntry { key: key.clone(), value };

            let provider_mut = self.fork_provider.db.provider_mut();
            provider_mut.tx().put::<tables::StorageChangeSet>(key, block_list)?;
            provider_mut.tx().put::<tables::StorageChangeHistory>(block, change_entry)?;
            provider_mut.commit()?;

            Ok(res)
        } else {
            Ok(None)
        }
    }
}

impl<Tx1: DbTx> StateProofProvider for HistoricalStateProvider<Tx1> {
    fn class_multiproof(&self, classes: Vec<ClassHash>) -> ProviderResult<katana_trie::MultiProof> {
        if self.provider.block() > self.fork_db.block_id {
            let mut tx = self.local_db.db().tx()?;
            let proofs = TrieDbFactory::new(&tx)
                .historical(self.provider.block())
                .ok_or(ProviderError::StateProofNotSupported)?
                .classes_trie()
                .multiproof(classes);
            tx.commit()?;
            Ok(proofs)
        } else {
            let result = self.fork_db.backend.get_classes_proofs(classes, self.provider.block())?;
            let proofs = result.expect("block should exist");

            Ok(proofs.classes_proof.nodes.into())
        }
    }

    fn contract_multiproof(
        &self,
        addresses: Vec<ContractAddress>,
    ) -> ProviderResult<katana_trie::MultiProof> {
        if self.provider.block() > self.fork_db.block_id {
            let mut tx = self.local_db.db().tx()?;
            let proofs = TrieDbFactory::new(&tx)
                .historical(self.provider.block())
                .ok_or(ProviderError::StateProofNotSupported)?
                .contracts_trie()
                .multiproof(addresses);
            tx.commit()?;
            Ok(proofs)
        } else {
            let result =
                self.fork_db.backend.get_contracts_proofs(addresses, self.provider.block())?;
            let proofs = result.expect("block should exist");

            Ok(proofs.classes_proof.nodes.into())
        }
    }

    fn storage_multiproof(
        &self,
        address: ContractAddress,
        storage_keys: Vec<StorageKey>,
    ) -> ProviderResult<katana_trie::MultiProof> {
        if self.provider.block() > self.fork_db.block_id {
            let mut tx = self.local_db.db().tx()?;
            let proofs = TrieDbFactory::new(&tx)
                .historical(self.provider.block())
                .ok_or(ProviderError::StateProofNotSupported)?
                .storages_trie(address)
                .multiproof(storage_keys);
            tx.commit()?;
            Ok(proofs)
        } else {
            let key = vec![ContractStorageKeys { address, keys: storage_keys }];
            let result = self.fork_db.backend.get_storages_proofs(key, self.provider.block())?;

            let mut proofs = result.expect("block should exist");
            let proofs = proofs.contracts_storage_proofs.nodes.pop().unwrap();

            Ok(proofs.into())
        }
    }
}

impl<Tx1: DbTx> StateRootProvider for HistoricalStateProvider<Tx1> {
    fn state_root(&self) -> ProviderResult<Felt> {
        match self.local_provider.state_root() {
            Ok(root) => Ok(root),

            Err(ProviderError::MissingBlockHeader(..)) => {
                let block_id = BlockHashOrNumber::from(self.local_provider.block());

                if let Some(header) = self.fork_provider.db.provider().header(block_id)? {
                    return Ok(header.state_root);
                }

                if self.fork_provider.fetch_historical_blocks(block_id)? {
                    let header = self.fork_provider.db.provider().header(block_id)?.unwrap();
                    Ok(header.state_root)
                } else {
                    Err(ProviderError::MissingBlockHeader(self.local_provider.block()))
                }
            }

            Err(err) => Err(err),
        }
    }

    fn classes_root(&self) -> ProviderResult<Felt> {
        if self.provider.block() > self.fork_db.block_id {
            let mut tx = self.local_db.db().tx()?;
            let root = TrieDbFactory::new(&tx)
                .historical(self.provider.block())
                .ok_or(ProviderError::StateProofNotSupported)?
                .classes_trie()
                .root();
            tx.commit()?;
            return Ok(root);
        }

        let result = self.fork_provider.backend.get_global_roots(self.local_provider.block())?;
        let roots = result.expect("block should exist");
        Ok(roots.global_roots.classes_tree_root)
    }

    fn contracts_root(&self) -> ProviderResult<Felt> {
        if self.provider.block() > self.fork_db.block_id {
            let mut tx = self.local_db.db().tx()?;
            let root = TrieDbFactory::new(&tx)
                .historical(self.provider.block())
                .ok_or(ProviderError::StateProofNotSupported)?
                .contracts_trie()
                .root();
            tx.commit()?;
            return Ok(root);
        }

        let result = self.fork_provider.backend.get_global_roots(self.local_provider.block())?;
        let roots = result.expect("block should exist");
        Ok(roots.global_roots.contracts_tree_root)
    }

    fn storage_root(&self, contract: ContractAddress) -> ProviderResult<Option<Felt>> {
        if self.provider.block() > self.fork_db.block_id {
            let mut tx = self.local_db.db().tx()?;
            let root = TrieDbFactory::new(&tx)
                .historical(self.provider.block())
                .ok_or(ProviderError::StateProofNotSupported)?
                .storages_trie(contract)
                .root();
            tx.commit()?;
            return Ok(Some(root));
        }

        let result =
            self.fork_provider.backend.get_storage_root(contract, self.local_provider.block())?;
        Ok(result)
    }
}

impl<Tx1: DbTxMut> StateWriter for ForkedProvider<Tx1> {
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

impl<Tx1: DbTxMut> ContractClassWriter for ForkedProvider<Tx1> {
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
