use katana_db::abstraction::{DbCursorMut, DbDupSortCursor, DbTx, DbTxMut};
use katana_db::models::contract::ContractInfoChangeList;
use katana_db::models::list::BlockList;
use katana_db::models::storage::{ContractStorageKey, StorageEntry};
use katana_db::tables;
use katana_db::trie::TrieDbFactory;
use katana_primitives::block::{BlockHashOrNumber, BlockNumber};
use katana_primitives::class::{ClassHash, CompiledClassHash, ContractClass};
use katana_primitives::contract::{
    ContractAddress, GenericContractInfo, Nonce, StorageKey, StorageValue,
};
use katana_primitives::Felt;
use katana_provider_api::block::BlockNumberProvider;
use katana_provider_api::contract::{ContractClassProvider, ContractClassWriter};
use katana_provider_api::state::{
    MultiProof, StateFactoryProvider, StateProofProvider, StateProvider, StateRootProvider,
    StateWriter,
};
use katana_provider_api::ProviderError;

use super::DbProvider;
use crate::ProviderResult;

impl<Tx: DbTxMut> StateWriter for DbProvider<Tx> {
    fn set_nonce(&self, address: ContractAddress, nonce: Nonce) -> ProviderResult<()> {
        let value = if let Some(info) = self.0.get::<tables::ContractInfo>(address)? {
            GenericContractInfo { nonce, ..info }
        } else {
            GenericContractInfo { nonce, ..Default::default() }
        };

        self.0.put::<tables::ContractInfo>(address, value)?;
        Ok(())
    }

    fn set_storage(
        &self,
        address: ContractAddress,
        storage_key: StorageKey,
        storage_value: StorageValue,
    ) -> ProviderResult<()> {
        let mut cursor = self.0.cursor_dup_mut::<tables::ContractStorage>()?;
        let entry = cursor.seek_by_key_subkey(address, storage_key)?;

        match entry {
            Some(entry) if entry.key == storage_key => {
                cursor.delete_current()?;
            }
            _ => {}
        }

        cursor.upsert(address, StorageEntry { key: storage_key, value: storage_value })?;
        Ok(())
    }

    fn set_class_hash_of_contract(
        &self,
        address: ContractAddress,
        class_hash: ClassHash,
    ) -> ProviderResult<()> {
        let value = if let Some(info) = self.0.get::<tables::ContractInfo>(address)? {
            GenericContractInfo { class_hash, ..info }
        } else {
            GenericContractInfo { class_hash, ..Default::default() }
        };

        self.0.put::<tables::ContractInfo>(address, value)?;
        Ok(())
    }
}

impl<Tx: DbTxMut> ContractClassWriter for DbProvider<Tx> {
    fn set_class(&self, hash: ClassHash, class: ContractClass) -> ProviderResult<()> {
        self.0.put::<tables::Classes>(hash, class.into())?;
        Ok(())
    }

    fn set_compiled_class_hash_of_class_hash(
        &self,
        hash: ClassHash,
        compiled_hash: CompiledClassHash,
    ) -> ProviderResult<()> {
        self.0.put::<tables::CompiledClassHashes>(hash, compiled_hash)?;
        Ok(())
    }
}

impl<Tx: DbTx> StateFactoryProvider for DbProvider<Tx> {
    fn latest(&self) -> ProviderResult<Box<dyn StateProvider>> {
        Ok(Box::new(LatestStateProvider(self.clone())))
    }

    fn historical(
        &self,
        block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<Box<dyn StateProvider>>> {
        let block_number = match block_id {
            BlockHashOrNumber::Num(num) => {
                let latest_num = self.latest_number()?;

                match num.cmp(&latest_num) {
                    std::cmp::Ordering::Less => Some(num),
                    std::cmp::Ordering::Greater => return Ok(None),
                    std::cmp::Ordering::Equal => return self.latest().map(Some),
                }
            }

            BlockHashOrNumber::Hash(hash) => self.block_number_by_hash(hash)?,
        };

        let Some(num) = block_number else { return Ok(None) };

        Ok(Some(Box::new(HistoricalStateProvider::new(self.0.clone(), num))))
    }
}

/// A state provider that provides the latest states from the database.
#[derive(Debug)]
pub(crate) struct LatestStateProvider<Tx: DbTx>(pub(crate) DbProvider<Tx>);

impl<Tx: DbTx> ContractClassProvider for LatestStateProvider<Tx> {
    fn class(&self, hash: ClassHash) -> ProviderResult<Option<ContractClass>> {
        Ok(self.0.get::<tables::Classes>(hash)?.map(|class| class.into()))
    }

    fn compiled_class_hash_of_class_hash(
        &self,
        hash: ClassHash,
    ) -> ProviderResult<Option<CompiledClassHash>> {
        let hash = self.0.get::<tables::CompiledClassHashes>(hash)?;
        Ok(hash)
    }
}

impl<Tx: DbTx> StateProvider for LatestStateProvider<Tx> {
    fn nonce(&self, address: ContractAddress) -> ProviderResult<Option<Nonce>> {
        let info = self.0.get::<tables::ContractInfo>(address)?;
        Ok(info.map(|info| info.nonce))
    }

    fn class_hash_of_contract(
        &self,
        address: ContractAddress,
    ) -> ProviderResult<Option<ClassHash>> {
        let info = self.0.get::<tables::ContractInfo>(address)?;
        Ok(info.map(|info| info.class_hash))
    }

    fn storage(
        &self,
        address: ContractAddress,
        storage_key: StorageKey,
    ) -> ProviderResult<Option<StorageValue>> {
        let mut cursor = self.0.cursor_dup::<tables::ContractStorage>()?;
        let entry = cursor.seek_by_key_subkey(address, storage_key)?;
        match entry {
            Some(entry) if entry.key == storage_key => Ok(Some(entry.value)),
            _ => Ok(None),
        }
    }
}

impl<Tx: DbTx> StateProofProvider for LatestStateProvider<Tx> {
    fn class_multiproof(&self, classes: Vec<ClassHash>) -> ProviderResult<MultiProof> {
        let factory = TrieDbFactory::new(self.0.tx().clone());
        let latest = self.0.latest_number()?;
        let trie = factory.classes_trie(latest);
        // We need the root index to generate proofs
        if let Some(root_idx) =
            factory.get_root_for_proofs(katana_db::models::trie::TrieType::Classes, latest)
        {
            trie.proofs(root_idx, &classes).map_err(|e| ProviderError::Other(e.to_string()))
        } else {
            Ok(vec![])
        }
    }

    fn contract_multiproof(&self, addresses: Vec<ContractAddress>) -> ProviderResult<MultiProof> {
        let factory = TrieDbFactory::new(self.0.tx().clone());
        let latest = self.0.latest_number()?;
        let trie = factory.contracts_trie(latest);
        if let Some(root_idx) =
            factory.get_root_for_proofs(katana_db::models::trie::TrieType::Contracts, latest)
        {
            trie.proofs(root_idx, &addresses).map_err(|e| ProviderError::Other(e.to_string()))
        } else {
            Ok(vec![])
        }
    }

    fn storage_multiproof(
        &self,
        address: ContractAddress,
        storage_keys: Vec<StorageKey>,
    ) -> ProviderResult<MultiProof> {
        let factory = TrieDbFactory::new(self.0.tx().clone());
        let latest = self.0.latest_number()?;
        let trie = factory.storages_trie(address, latest);
        if let Some(root_idx) = factory.get_storage_root_for_proofs(address, latest) {
            trie.proofs(root_idx, &storage_keys).map_err(|e| ProviderError::Other(e.to_string()))
        } else {
            Ok(vec![])
        }
    }
}

impl<Tx: DbTx> StateRootProvider for LatestStateProvider<Tx> {
    fn classes_root(&self) -> ProviderResult<Felt> {
        let factory = TrieDbFactory::new(self.0.tx().clone());
        let latest = self.0.latest_number()?;
        let trie = factory.classes_trie(latest);
        trie.root_hash().map_err(|e| ProviderError::Other(e.to_string()))
    }

    fn contracts_root(&self) -> ProviderResult<Felt> {
        let factory = TrieDbFactory::new(self.0.tx().clone());
        let latest = self.0.latest_number()?;
        let trie = factory.contracts_trie(latest);
        trie.root_hash().map_err(|e| ProviderError::Other(e.to_string()))
    }

    fn storage_root(&self, contract: ContractAddress) -> ProviderResult<Option<Felt>> {
        let factory = TrieDbFactory::new(self.0.tx().clone());
        let latest = self.0.latest_number()?;
        let trie = factory.storages_trie(contract, latest);
        let root = trie.root_hash().map_err(|e| ProviderError::Other(e.to_string()))?;
        Ok(Some(root))
    }
}

/// A historical state provider.
#[derive(Debug)]
pub(crate) struct HistoricalStateProvider<Tx: DbTx> {
    /// The database transaction used to read the database.
    tx: Tx,
    /// The block number of the state.
    block_number: BlockNumber,
}

impl<Tx: DbTx> HistoricalStateProvider<Tx> {
    pub fn new(tx: Tx, block_number: BlockNumber) -> Self {
        Self { tx, block_number }
    }

    pub fn tx(&self) -> &Tx {
        &self.tx
    }

    /// The block number this state provider is pinned to.
    pub fn block(&self) -> BlockNumber {
        self.block_number
    }

    /// Check if the class was declared before the pinned block number.
    fn is_class_declared_before_block(&self, hash: ClassHash) -> ProviderResult<bool> {
        let decl_block_num = self.tx.get::<tables::ClassDeclarationBlock>(hash)?;
        let is_declared = decl_block_num.is_some_and(|num| num <= self.block_number);
        Ok(is_declared)
    }
}

impl<Tx: DbTx> ContractClassProvider for HistoricalStateProvider<Tx> {
    fn class(&self, hash: ClassHash) -> ProviderResult<Option<ContractClass>> {
        if self.is_class_declared_before_block(hash)? {
            Ok(self.tx.get::<tables::Classes>(hash)?.map(Into::into))
        } else {
            Ok(None)
        }
    }

    fn compiled_class_hash_of_class_hash(
        &self,
        hash: ClassHash,
    ) -> ProviderResult<Option<CompiledClassHash>> {
        if self.is_class_declared_before_block(hash)? {
            Ok(self.tx.get::<tables::CompiledClassHashes>(hash)?)
        } else {
            Ok(None)
        }
    }
}

impl<Tx: DbTx> StateProvider for HistoricalStateProvider<Tx> {
    fn nonce(&self, address: ContractAddress) -> ProviderResult<Option<Nonce>> {
        let change_list = self.tx.get::<tables::ContractInfoChangeSet>(address)?;

        if let Some(num) = change_list
            .and_then(|entry| recent_change_from_block(self.block_number, &entry.nonce_change_list))
        {
            let mut cursor = self.tx.cursor_dup::<tables::NonceChangeHistory>()?;
            let entry = cursor.seek_by_key_subkey(num, address)?.ok_or(
                ProviderError::MissingContractNonceChangeEntry {
                    block: num,
                    contract_address: address,
                },
            )?;

            if entry.contract_address == address {
                return Ok(Some(entry.nonce));
            }
        }

        Ok(None)
    }

    fn class_hash_of_contract(
        &self,
        address: ContractAddress,
    ) -> ProviderResult<Option<ClassHash>> {
        let change_list: Option<ContractInfoChangeList> =
            self.tx.get::<tables::ContractInfoChangeSet>(address)?;

        if let Some(num) = change_list
            .and_then(|entry| recent_change_from_block(self.block_number, &entry.class_change_list))
        {
            let mut cursor = self.tx.cursor_dup::<tables::ClassChangeHistory>()?;
            let entry = cursor.seek_by_key_subkey(num, address)?.ok_or(
                ProviderError::MissingContractClassChangeEntry {
                    block: num,
                    contract_address: address,
                },
            )?;

            if entry.contract_address == address {
                return Ok(Some(entry.class_hash));
            }
        }

        Ok(None)
    }

    fn storage(
        &self,
        address: ContractAddress,
        storage_key: StorageKey,
    ) -> ProviderResult<Option<StorageValue>> {
        let key = ContractStorageKey { contract_address: address, key: storage_key };
        let block_list = self.tx.get::<tables::StorageChangeSet>(key.clone())?;

        if let Some(num) =
            block_list.and_then(|list| recent_change_from_block(self.block_number, &list))
        {
            let mut cursor = self.tx.cursor_dup::<tables::StorageChangeHistory>()?;
            let entry = cursor.seek_by_key_subkey(num, key)?.ok_or(
                ProviderError::MissingStorageChangeEntry {
                    block: num,
                    storage_key,
                    contract_address: address,
                },
            )?;

            if entry.key.contract_address == address && entry.key.key == storage_key {
                return Ok(Some(entry.value));
            }
        }

        Ok(None)
    }
}

impl<Tx: DbTx> StateProofProvider for HistoricalStateProvider<Tx> {
    fn class_multiproof(&self, classes: Vec<ClassHash>) -> ProviderResult<MultiProof> {
        let factory = TrieDbFactory::new(self.tx().clone());
        let trie = factory.classes_trie(self.block_number);
        if let Some(root_idx) = factory
            .get_root_for_proofs(katana_db::models::trie::TrieType::Classes, self.block_number)
        {
            trie.proofs(root_idx, &classes).map_err(|e| ProviderError::Other(e.to_string()))
        } else {
            Ok(vec![])
        }
    }

    fn contract_multiproof(&self, addresses: Vec<ContractAddress>) -> ProviderResult<MultiProof> {
        let factory = TrieDbFactory::new(self.tx().clone());
        let trie = factory.contracts_trie(self.block_number);
        if let Some(root_idx) = factory
            .get_root_for_proofs(katana_db::models::trie::TrieType::Contracts, self.block_number)
        {
            trie.proofs(root_idx, &addresses).map_err(|e| ProviderError::Other(e.to_string()))
        } else {
            Ok(vec![])
        }
    }

    fn storage_multiproof(
        &self,
        address: ContractAddress,
        storage_keys: Vec<StorageKey>,
    ) -> ProviderResult<MultiProof> {
        let factory = TrieDbFactory::new(self.tx().clone());
        let trie = factory.storages_trie(address, self.block_number);
        if let Some(root_idx) = factory.get_storage_root_for_proofs(address, self.block_number) {
            trie.proofs(root_idx, &storage_keys).map_err(|e| ProviderError::Other(e.to_string()))
        } else {
            Ok(vec![])
        }
    }
}

impl<Tx: DbTx> StateRootProvider for HistoricalStateProvider<Tx> {
    fn classes_root(&self) -> ProviderResult<katana_primitives::Felt> {
        let factory = TrieDbFactory::new(self.tx().clone());
        let trie = factory.classes_trie(self.block_number);
        trie.root_hash().map_err(|e| ProviderError::Other(e.to_string()))
    }

    fn contracts_root(&self) -> ProviderResult<katana_primitives::Felt> {
        let factory = TrieDbFactory::new(self.tx().clone());
        let trie = factory.contracts_trie(self.block_number);
        trie.root_hash().map_err(|e| ProviderError::Other(e.to_string()))
    }

    fn storage_root(&self, contract: ContractAddress) -> ProviderResult<Option<Felt>> {
        let factory = TrieDbFactory::new(self.tx().clone());
        let trie = factory.storages_trie(contract, self.block_number);
        let root = trie.root_hash().map_err(|e| ProviderError::Other(e.to_string()))?;
        Ok(Some(root))
    }

    fn state_root(&self) -> ProviderResult<katana_primitives::Felt> {
        let header = self
            .tx
            .get::<tables::Headers>(self.block_number)?
            .ok_or(ProviderError::MissingBlockHeader(self.block_number))?;
        let header: katana_primitives::block::Header = header.into();
        Ok(header.state_root)
    }
}

/// This is a helper function for getting the block number of the most
/// recent change that occurred relative to the given block number.
///
/// ## Arguments
///
/// * `block_list`: A list of block numbers where a change in value occur.
fn recent_change_from_block(
    block_number: BlockNumber,
    block_list: &BlockList,
) -> Option<BlockNumber> {
    // if the rank is 0, then it's either;
    // 1. the list is empty
    // 2. there are no prior changes occured before/at `block_number`
    let rank = block_list.rank(block_number);
    if rank == 0 {
        None
    } else {
        block_list.select(rank - 1)
    }
}

#[cfg(test)]
mod tests {
    use katana_db::models::list::BlockList;

    #[rstest::rstest]
    #[case(0, None)]
    #[case(1, Some(1))]
    #[case(3, Some(2))]
    #[case(5, Some(5))]
    #[case(9, Some(6))]
    #[case(10, Some(10))]
    #[case(11, Some(10))]
    fn position_of_most_recent_block_in_block_list(
        #[case] block_num: u64,
        #[case] expected_block_num: Option<u64>,
    ) {
        let list = BlockList::from([1, 2, 5, 6, 10]);
        let actual_block_num = super::recent_change_from_block(block_num, &list);
        assert_eq!(actual_block_num, expected_block_num);
    }
}
