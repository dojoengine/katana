use std::cmp::Ordering;
use std::sync::Arc;

use katana_db::abstraction::{Database, DbTx, DbTxMut};
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
use katana_rpc_types::ContractStorageKeys;

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

impl<Db: Database> StateProofProvider for LatestStateProvider<Db> {
    fn class_multiproof(&self, classes: Vec<ClassHash>) -> ProviderResult<katana_trie::MultiProof> {
        let fork_point = self.fork_db.block_id;
        let latest_block_number = match self.db.latest_number() {
            Ok(num) => num,
            // return the fork block number if local db return this error. this can only happen whne
            // the ForkedProvider is constructed without inserting any locally produced
            // blocks.
            Err(ProviderError::MissingLatestBlockNumber) => self.fork_db.block_id,
            Err(err) => return Err(err),
        };

        if latest_block_number == fork_point {
            let result = self.fork_db.backend.get_classes_proofs(classes, fork_point)?;
            let proofs = result.expect("proofs should exist for block");

            Ok(proofs.classes_proof.nodes.into())
        } else {
            Err(ProviderError::StateProofNotSupported)
        }
    }

    fn contract_multiproof(
        &self,
        addresses: Vec<ContractAddress>,
    ) -> ProviderResult<katana_trie::MultiProof> {
        let fork_point = self.fork_db.block_id;
        let latest_block_number = self.db.latest_number()?;

        if latest_block_number == fork_point {
            let result = self.fork_db.backend.get_contracts_proofs(addresses, fork_point)?;
            let proofs = result.expect("proofs should exist for block");

            Ok(proofs.classes_proof.nodes.into())
        } else {
            Err(ProviderError::StateProofNotSupported)
        }
    }

    fn storage_multiproof(
        &self,
        address: ContractAddress,
        storage_keys: Vec<StorageKey>,
    ) -> ProviderResult<katana_trie::MultiProof> {
        let fork_point = self.fork_db.block_id;
        let latest_block_number = self.db.latest_number()?;

        if latest_block_number == fork_point {
            let key = vec![ContractStorageKeys { address, keys: storage_keys }];
            let result = self.fork_db.backend.get_storages_proofs(key, fork_point)?;

            let mut proofs = result.expect("proofs should exist for block");
            let proofs = proofs.contracts_storage_proofs.nodes.pop().unwrap();

            Ok(proofs.into())
        } else {
            Err(ProviderError::StateProofNotSupported)
        }
    }
}

impl<Db: Database> StateRootProvider for LatestStateProvider<Db> {
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
        // we don't have a way to construct state proofs for locally generated state yet
        if self.provider.block() > self.fork_db.block_id {
            return Err(ProviderError::StateProofNotSupported);
        }

        let result = self.fork_db.backend.get_classes_proofs(classes, self.fork_db.block_id)?;
        let proofs = result.expect("block should exist");

        Ok(proofs.classes_proof.nodes.into())
    }

    fn contract_multiproof(
        &self,
        addresses: Vec<ContractAddress>,
    ) -> ProviderResult<katana_trie::MultiProof> {
        // we don't have a way to construct state proofs for locally generated state yet
        if self.provider.block() > self.fork_db.block_id {
            return Err(ProviderError::StateProofNotSupported);
        }

        let result = self.fork_db.backend.get_contracts_proofs(addresses, self.fork_db.block_id)?;
        let proofs = result.expect("block should exist");

        Ok(proofs.classes_proof.nodes.into())
    }

    fn storage_multiproof(
        &self,
        address: ContractAddress,
        storage_keys: Vec<StorageKey>,
    ) -> ProviderResult<katana_trie::MultiProof> {
        // we don't have a way to construct state proofs for locally generated state yet
        if self.provider.block() > self.fork_db.block_id {
            return Err(ProviderError::StateProofNotSupported);
        }

        let key = vec![ContractStorageKeys { address, keys: storage_keys }];
        let result = self.fork_db.backend.get_storages_proofs(key, self.fork_db.block_id)?;

        let mut proofs = result.expect("block should exist");
        let proofs = proofs.contracts_storage_proofs.nodes.pop().unwrap();

        Ok(proofs.into())
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
        // note: we are not computing the state trie correctly for block post-fork
        if self.provider.block() > self.fork_db.block_id {
            return Ok(Felt::ZERO);
        }

        let result = self.fork_db.backend.get_global_roots(self.fork_db.block_id)?;
        let roots = result.expect("block should exist");
        Ok(roots.global_roots.classes_tree_root)
    }

    fn contracts_root(&self) -> ProviderResult<Felt> {
        // note: we are not computing the state trie correctly for block post-fork
        if self.provider.block() > self.fork_db.block_id {
            return Ok(Felt::ZERO);
        }

        let result = self.fork_db.backend.get_global_roots(self.fork_db.block_id)?;
        let roots = result.expect("block should exist");
        Ok(roots.global_roots.contracts_tree_root)
    }

    fn storage_root(&self, contract: ContractAddress) -> ProviderResult<Option<Felt>> {
        // note: we are not computing the state trie correctly for block post-fork
        if self.provider.block() > self.fork_db.block_id {
            return Ok(None);
        }

        let result = self.fork_db.backend.get_storage_root(contract, self.fork_db.block_id)?;
        Ok(result)
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
