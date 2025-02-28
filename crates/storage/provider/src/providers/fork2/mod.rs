use std::fmt;
use std::sync::Arc;

use katana_db::abstraction::{Database, DbTx};
use katana_db::mdbx::DbEnv;
use katana_primitives::block::BlockHashOrNumber;
use katana_primitives::class::{ClassHash, CompiledClassHash, ContractClass};
use katana_primitives::contract::{Nonce, StorageKey, StorageValue};
use katana_primitives::{ContractAddress, Felt};
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;

use super::db;
use super::fork::backend::{Backend, BackendHandle};
use crate::traits::contract::ContractClassProvider;
use crate::traits::state::{
    StateFactoryProvider, StateProofProvider, StateProvider, StateRootProvider,
};
use crate::ProviderResult;

#[derive(Debug)]
pub struct ForkedProvider<Db: Database = DbEnv> {
    db: Db,
    backend: BackendHandle,
}

impl<Db: Database> ForkedProvider<Db> {
    pub fn new(
        db: Db,
        provider: Arc<JsonRpcClient<HttpTransport>>,
        block_id: BlockHashOrNumber,
    ) -> Self {
        let backend = Backend::new(provider, block_id).expect("failed to create backend");
        Self { db, backend }
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    pub fn backend(&self) -> &BackendHandle {
        &self.backend
    }
}

impl<Db: Database> StateFactoryProvider for ForkedProvider<Db> {
    fn latest(&self) -> ProviderResult<Box<dyn StateProvider>> {
        // Ok(Box::new(self::state::LatestStateProvider(Arc::clone(&self.state))))
        todo!()
    }

    fn historical(
        &self,
        block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<Box<dyn StateProvider>>> {
        // let block_num = match block_id {
        //     BlockHashOrNumber::Num(num) => Some(num),
        //     BlockHashOrNumber::Hash(hash) => self.block_number_by_hash(hash)?,
        // };

        // let provider @ Some(_) = block_num.and_then(|num| {
        //     self.historical_states
        //         .read()
        //         .get(&num)
        //         .cloned()
        //         .map(|provider| Box::new(provider) as Box<dyn StateProvider>)
        // }) else {
        //     return Ok(None);
        // };

        // Ok(provider)
        todo!()
    }
}

#[derive(Debug)]
struct LatestStateProvider<Tx: DbTx> {
    backend: BackendHandle,
    provider: db::state::LatestStateProvider<Tx>,
}

impl<Tx> ContractClassProvider for LatestStateProvider<Tx>
where
    Tx: DbTx + Send + Sync,
{
    fn class(&self, hash: ClassHash) -> ProviderResult<Option<ContractClass>> {
        if let Some(class) = self.provider.class(hash)? {
            Ok(Some(class))
        } else {
            self.backend.get_class_at(hash)?;
            Ok(None)
        }
    }

    fn compiled_class_hash_of_class_hash(
        &self,
        hash: ClassHash,
    ) -> ProviderResult<Option<CompiledClassHash>> {
        self.provider.compiled_class_hash_of_class_hash(hash)
    }
}

impl<Tx> StateProvider for LatestStateProvider<Tx>
where
    Tx: DbTx + fmt::Debug + Send + Sync,
{
    fn nonce(&self, address: ContractAddress) -> ProviderResult<Option<Nonce>> {
        self.provider.nonce(address)
    }

    fn class_hash_of_contract(
        &self,
        address: ContractAddress,
    ) -> ProviderResult<Option<ClassHash>> {
        self.provider.class_hash_of_contract(address)
    }

    fn storage(
        &self,
        address: ContractAddress,
        storage_key: StorageKey,
    ) -> ProviderResult<Option<StorageValue>> {
        self.provider.storage(address, storage_key)
    }
}
impl<Tx> StateProofProvider for LatestStateProvider<Tx>
where
    Tx: DbTx + fmt::Debug + Send + Sync,
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
    Tx: DbTx + fmt::Debug + Send + Sync,
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
