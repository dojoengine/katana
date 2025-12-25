pub mod state;
pub mod trie;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::ops::{Deref, Range, RangeInclusive};

use katana_db::abstraction::{DbCursor, DbCursorMut, DbDupSortCursor, DbTx, DbTxMut};
use katana_db::error::DatabaseError;
use katana_db::models::block::StoredBlockBodyIndices;
use katana_db::models::class::MigratedCompiledClassHash;
use katana_db::models::contract::{
    ContractClassChange, ContractClassChangeType, ContractInfoChangeList, ContractNonceChange,
};
use katana_db::models::list::BlockList;
use katana_db::models::stage::{ExecutionCheckpoint, PruningCheckpoint};
use katana_db::models::storage::{ContractStorageEntry, ContractStorageKey, StorageEntry};
use katana_db::models::{VersionedHeader, VersionedTx};
use katana_db::tables::{self, DupSort, Table};
use katana_db::utils::KeyValue;
use katana_primitives::block::{
    Block, BlockHash, BlockHashOrNumber, BlockNumber, BlockWithTxHashes, FinalityStatus, Header,
    SealedBlockWithStatus,
};
use katana_primitives::class::{ClassHash, CompiledClassHash};
use katana_primitives::contract::{
    ContractAddress, GenericContractInfo, Nonce, StorageKey, StorageValue,
};
use katana_primitives::env::BlockEnv;
use katana_primitives::execution::TypedTransactionExecutionInfo;
use katana_primitives::receipt::Receipt;
use katana_primitives::state::{StateUpdates, StateUpdatesWithClasses};
use katana_primitives::transaction::{TxHash, TxNumber, TxWithHash};
use katana_provider_api::block::{
    BlockHashProvider, BlockIdReader, BlockNumberProvider, BlockProvider, BlockStatusProvider,
    BlockWriter, HeaderProvider,
};
use katana_provider_api::env::BlockEnvProvider;
use katana_provider_api::stage::StageCheckpointProvider;
use katana_provider_api::state_update::StateUpdateProvider;
use katana_provider_api::transaction::{
    ReceiptProvider, TransactionProvider, TransactionStatusProvider, TransactionTraceProvider,
    TransactionsProviderExt,
};
use katana_provider_api::ProviderError;

use crate::{MutableProvider, ProviderResult};

/// A provider implementation that uses a persistent database as the backend.
// TODO: remove the default generic type
#[derive(Clone)]
pub struct DbProvider<Tx: DbTx>(Tx);

impl<Tx: DbTx> Deref for DbProvider<Tx> {
    type Target = Tx;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<Tx: DbTx + Debug> Debug for DbProvider<Tx> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("DbProvider").field(&self.0).finish()
    }
}

impl<Tx: DbTx> DbProvider<Tx> {
    /// Creates a new [`DbProvider`] from the given [`DbEnv`].
    pub fn new(db: Tx) -> Self {
        Self(db)
    }

    /// Returns the [`DbTx`] associated with this provider.
    pub fn tx(&self) -> &Tx {
        &self.0
    }
}

impl<Tx: DbTxMut> MutableProvider for DbProvider<Tx> {
    fn commit(self) -> ProviderResult<()> {
        let _ = self.0.commit()?;
        Ok(())
    }
}

impl<Tx: DbTx> BlockNumberProvider for DbProvider<Tx> {
    fn block_number_by_hash(&self, hash: BlockHash) -> ProviderResult<Option<BlockNumber>> {
        let block_num = self.0.get::<tables::BlockNumbers>(hash)?;
        Ok(block_num)
    }

    fn latest_number(&self) -> ProviderResult<BlockNumber> {
        let res = self.0.cursor::<tables::BlockHashes>()?.last()?.map(|(num, _)| num);
        let total_blocks = res.ok_or(ProviderError::MissingLatestBlockNumber)?;
        Ok(total_blocks)
    }
}

impl<Tx: DbTx> BlockIdReader for DbProvider<Tx> {}

impl<Tx: DbTx> BlockHashProvider for DbProvider<Tx> {
    fn latest_hash(&self) -> ProviderResult<BlockHash> {
        let latest_block = self.latest_number()?;
        let latest_hash = self.0.get::<tables::BlockHashes>(latest_block)?;
        latest_hash.ok_or(ProviderError::MissingLatestBlockHash)
    }

    fn block_hash_by_num(&self, num: BlockNumber) -> ProviderResult<Option<BlockHash>> {
        Ok(self.0.get::<tables::BlockHashes>(num)?)
    }
}

impl<Tx: DbTx> HeaderProvider for DbProvider<Tx> {
    fn header(&self, id: BlockHashOrNumber) -> ProviderResult<Option<Header>> {
        match id {
            BlockHashOrNumber::Num(num) => {
                let header = self.0.get::<tables::Headers>(num)?.map(Header::from);
                Ok(header)
            }

            BlockHashOrNumber::Hash(hash) => {
                if let Some(num) = self.0.get::<tables::BlockNumbers>(hash)? {
                    let header = self
                        .0
                        .get::<tables::Headers>(num)?
                        .ok_or(ProviderError::MissingBlockHeader(num))?;

                    Ok(Some(header.into()))
                } else {
                    Ok(None)
                }
            }
        }
    }
}

impl<Tx: DbTx> BlockProvider for DbProvider<Tx> {
    fn block_body_indices(
        &self,
        id: BlockHashOrNumber,
    ) -> ProviderResult<Option<StoredBlockBodyIndices>> {
        let block_num = match id {
            BlockHashOrNumber::Num(num) => Some(num),
            BlockHashOrNumber::Hash(hash) => self.0.get::<tables::BlockNumbers>(hash)?,
        };

        if let Some(num) = block_num {
            let indices = self.0.get::<tables::BlockBodyIndices>(num)?;
            Ok(indices)
        } else {
            Ok(None)
        }
    }

    fn block(&self, id: BlockHashOrNumber) -> ProviderResult<Option<Block>> {
        if let Some(header) = self.header(id)? {
            let res = self.transactions_by_block(id)?;
            let body = res.ok_or(ProviderError::MissingBlockTxs(header.number))?;
            Ok(Some(Block { header, body }))
        } else {
            Ok(None)
        }
    }

    fn block_with_tx_hashes(
        &self,
        id: BlockHashOrNumber,
    ) -> ProviderResult<Option<BlockWithTxHashes>> {
        let block_num = match id {
            BlockHashOrNumber::Num(num) => Some(num),
            BlockHashOrNumber::Hash(hash) => self.0.get::<tables::BlockNumbers>(hash)?,
        };

        let Some(block_num) = block_num else { return Ok(None) };

        if let Some(header) = self.0.get::<tables::Headers>(block_num)? {
            let res = self.0.get::<tables::BlockBodyIndices>(block_num)?;
            let body_indices = res.ok_or(ProviderError::MissingBlockTxs(block_num))?;

            let body = self.transaction_hashes_in_range(Range::from(body_indices))?;
            let block = BlockWithTxHashes { header: header.into(), body };

            Ok(Some(block))
        } else {
            Ok(None)
        }
    }

    fn blocks_in_range(&self, range: RangeInclusive<u64>) -> ProviderResult<Vec<Block>> {
        let total = range.end().saturating_sub(*range.start()) + 1;
        let mut blocks = Vec::with_capacity(total as usize);

        for num in range {
            if let Some(header) = self.0.get::<tables::Headers>(num)? {
                let res = self.0.get::<tables::BlockBodyIndices>(num)?;
                let body_indices = res.ok_or(ProviderError::MissingBlockBodyIndices(num))?;

                let body = self.transaction_in_range(Range::from(body_indices))?;
                blocks.push(Block { header: header.into(), body })
            }
        }

        Ok(blocks)
    }
}

impl<Tx: DbTx> BlockStatusProvider for DbProvider<Tx> {
    fn block_status(&self, id: BlockHashOrNumber) -> ProviderResult<Option<FinalityStatus>> {
        match id {
            BlockHashOrNumber::Num(num) => {
                let status = self.0.get::<tables::BlockStatusses>(num)?;
                Ok(status)
            }

            BlockHashOrNumber::Hash(hash) => {
                if let Some(num) = self.block_number_by_hash(hash)? {
                    let res = self.0.get::<tables::BlockStatusses>(num)?;
                    let status = res.ok_or(ProviderError::MissingBlockStatus(num))?;
                    Ok(Some(status))
                } else {
                    Ok(None)
                }
            }
        }
    }
}

// A helper function that iterates over all entries in a dupsort table and collects the
// results into `V`. If `key` is not found, `V::default()` is returned.
fn dup_entries<Tx, Tb, V, T>(
    db_tx: &Tx,
    key: <Tb as Table>::Key,
    mut f: impl FnMut(Result<KeyValue<Tb>, DatabaseError>) -> ProviderResult<Option<T>>,
) -> ProviderResult<V>
where
    Tx: DbTx,
    Tb: DupSort + Debug,
    V: FromIterator<T> + Default,
{
    Ok(db_tx
        .cursor_dup::<Tb>()?
        .walk_dup(Some(key), None)?
        .map(|walker| walker.filter_map(|i| f(i).transpose()).collect::<ProviderResult<V>>())
        .transpose()?
        .unwrap_or_default())
}

impl<Tx: DbTx> StateUpdateProvider for DbProvider<Tx> {
    fn state_update(&self, block_id: BlockHashOrNumber) -> ProviderResult<Option<StateUpdates>> {
        let block_num = self.block_number_by_id(block_id)?;

        if let Some(block_num) = block_num {
            let nonce_updates = dup_entries::<
                Tx,
                tables::NonceChangeHistory,
                BTreeMap<ContractAddress, Nonce>,
                _,
            >(&self.0, block_num, |entry| {
                let (_, ContractNonceChange { contract_address, nonce }) = entry?;
                Ok(Some((contract_address, nonce)))
            })?;

            let deployed_contracts = dup_entries::<
                Tx,
                tables::ClassChangeHistory,
                BTreeMap<ContractAddress, ClassHash>,
                _,
            >(&self.0, block_num, |entry| {
                let (_, ContractClassChange { r#type, contract_address, class_hash }) = entry?;

                if r#type == ContractClassChangeType::Deployed {
                    Ok(Some((contract_address, class_hash)))
                } else {
                    Ok(None)
                }
            })?;

            let replaced_classes = dup_entries::<
                Tx,
                tables::ClassChangeHistory,
                BTreeMap<ContractAddress, ClassHash>,
                _,
            >(&self.0, block_num, |entry| {
                let (_, ContractClassChange { r#type, contract_address, class_hash }) = entry?;

                if r#type == ContractClassChangeType::Replaced {
                    Ok(Some((contract_address, class_hash)))
                } else {
                    Ok(None)
                }
            })?;

            let mut declared_classes = BTreeMap::new();
            let mut deprecated_declared_classes = BTreeSet::new();

            if let Some(block_entries) =
                self.0.cursor_dup::<tables::ClassDeclarations>()?.walk_dup(Some(block_num), None)?
            {
                for entry in block_entries {
                    let (_, class_hash) = entry?;
                    match self.0.get::<tables::CompiledClassHashes>(class_hash)? {
                        Some(compiled_hash) => {
                            declared_classes.insert(class_hash, compiled_hash);
                        }
                        None => {
                            deprecated_declared_classes.insert(class_hash);
                        }
                    }
                }
            }

            let migrated_compiled_classes = dup_entries::<
                Tx,
                tables::MigratedCompiledClassHashes,
                BTreeMap<ClassHash, CompiledClassHash>,
                _,
            >(&self.0, block_num, |entry| {
                let (_, MigratedCompiledClassHash { class_hash, compiled_class_hash }) = entry?;
                Ok(Some((class_hash, compiled_class_hash)))
            })?;

            let storage_updates = {
                let entries = dup_entries::<
                    Tx,
                    tables::StorageChangeHistory,
                    Vec<(ContractAddress, (StorageKey, StorageValue))>,
                    _,
                >(&self.0, block_num, |entry| {
                    let (_, ContractStorageEntry { key, value }) = entry?;
                    Ok(Some((key.contract_address, (key.key, value))))
                })?;

                let mut map: BTreeMap<_, BTreeMap<StorageKey, StorageValue>> = BTreeMap::new();

                entries.into_iter().for_each(|(addr, (key, value))| {
                    map.entry(addr).or_default().insert(key, value);
                });

                map
            };

            Ok(Some(StateUpdates {
                nonce_updates,
                storage_updates,
                deployed_contracts,
                declared_classes,
                replaced_classes,
                deprecated_declared_classes,
                migrated_compiled_classes,
            }))
        } else {
            Ok(None)
        }
    }

    fn declared_classes(
        &self,
        block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<BTreeMap<ClassHash, CompiledClassHash>>> {
        let block_num = self.block_number_by_id(block_id)?;

        if let Some(block_num) = block_num {
            let declared_classes = dup_entries::<
                Tx,
                tables::ClassDeclarations,
                BTreeMap<ClassHash, CompiledClassHash>,
                _,
            >(&self.0, block_num, |entry| {
                let (_, class_hash) = entry?;

                if let Some(compiled_hash) =
                    self.0.get::<tables::CompiledClassHashes>(class_hash)?
                {
                    Ok(Some((class_hash, compiled_hash)))
                } else {
                    Ok(None)
                }
            })?;

            Ok(Some(declared_classes))
        } else {
            Ok(None)
        }
    }

    fn deployed_contracts(
        &self,
        block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<BTreeMap<ContractAddress, ClassHash>>> {
        let block_num = self.block_number_by_id(block_id)?;

        if let Some(block_num) = block_num {
            let deployed_contracts = dup_entries::<
                Tx,
                tables::ClassChangeHistory,
                BTreeMap<ContractAddress, ClassHash>,
                _,
            >(&self.0, block_num, |entry| {
                let (_, ContractClassChange { r#type, contract_address, class_hash }) = entry?;
                if r#type == ContractClassChangeType::Deployed {
                    Ok(Some((contract_address, class_hash)))
                } else {
                    Ok(None)
                }
            })?;

            Ok(Some(deployed_contracts))
        } else {
            Ok(None)
        }
    }
}

impl<Tx: DbTx> TransactionProvider for DbProvider<Tx> {
    fn transaction_by_hash(&self, hash: TxHash) -> ProviderResult<Option<TxWithHash>> {
        if let Some(num) = self.0.get::<tables::TxNumbers>(hash)? {
            let res = self.0.get::<tables::Transactions>(num)?;
            let transaction = res.ok_or(ProviderError::MissingTx(num))?;
            Ok(Some(TxWithHash { hash, transaction: transaction.into() }))
        } else {
            Ok(None)
        }
    }

    fn transactions_by_block(
        &self,
        block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<Vec<TxWithHash>>> {
        if let Some(indices) = self.block_body_indices(block_id)? {
            Ok(Some(self.transaction_in_range(Range::from(indices))?))
        } else {
            Ok(None)
        }
    }

    fn transaction_in_range(&self, range: Range<TxNumber>) -> ProviderResult<Vec<TxWithHash>> {
        let total = range.end.saturating_sub(range.start);
        let mut transactions = Vec::with_capacity(total as usize);

        for i in range {
            if let Some(transaction) = self.0.get::<tables::Transactions>(i)? {
                let res = self.0.get::<tables::TxHashes>(i)?;
                let hash = res.ok_or(ProviderError::MissingTxHash(i))?;
                transactions.push(TxWithHash { hash, transaction: transaction.into() });
            };
        }

        Ok(transactions)
    }

    fn transaction_block_num_and_hash(
        &self,
        hash: TxHash,
    ) -> ProviderResult<Option<(BlockNumber, BlockHash)>> {
        if let Some(num) = self.0.get::<tables::TxNumbers>(hash)? {
            let block_num =
                self.0.get::<tables::TxBlocks>(num)?.ok_or(ProviderError::MissingTxBlock(num))?;

            let res = self.0.get::<tables::BlockHashes>(block_num)?;
            let block_hash = res.ok_or(ProviderError::MissingBlockHash(num))?;

            Ok(Some((block_num, block_hash)))
        } else {
            Ok(None)
        }
    }

    fn transaction_by_block_and_idx(
        &self,
        block_id: BlockHashOrNumber,
        idx: u64,
    ) -> ProviderResult<Option<TxWithHash>> {
        match self.block_body_indices(block_id)? {
            // make sure the requested idx is within the range of the block tx count
            Some(indices) if idx < indices.tx_count => {
                let num = indices.tx_offset + idx;

                let res = self.0.get::<tables::TxHashes>(num)?;
                let hash = res.ok_or(ProviderError::MissingTxHash(num))?;

                let res = self.0.get::<tables::Transactions>(num)?;
                let transaction = res.ok_or(ProviderError::MissingTx(num))?;

                Ok(Some(TxWithHash { hash, transaction: transaction.into() }))
            }

            _ => Ok(None),
        }
    }

    fn transaction_count_by_block(
        &self,
        block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<u64>> {
        if let Some(indices) = self.block_body_indices(block_id)? {
            Ok(Some(indices.tx_count))
        } else {
            Ok(None)
        }
    }
}

impl<Tx: DbTx> TransactionsProviderExt for DbProvider<Tx> {
    fn transaction_hashes_in_range(&self, range: Range<TxNumber>) -> ProviderResult<Vec<TxHash>> {
        let total = range.end.saturating_sub(range.start);
        let mut hashes = Vec::with_capacity(total as usize);

        for i in range {
            if let Some(hash) = self.0.get::<tables::TxHashes>(i)? {
                hashes.push(hash);
            }
        }

        Ok(hashes)
    }

    fn total_transactions(&self) -> ProviderResult<usize> {
        Ok(self.0.entries::<tables::Transactions>()?)
    }
}

impl<Tx: DbTx> TransactionStatusProvider for DbProvider<Tx> {
    fn transaction_status(&self, hash: TxHash) -> ProviderResult<Option<FinalityStatus>> {
        if let Some(tx_num) = self.0.get::<tables::TxNumbers>(hash)? {
            let res = self.0.get::<tables::TxBlocks>(tx_num)?;
            let block_num = res.ok_or(ProviderError::MissingTxBlock(tx_num))?;

            let res = self.0.get::<tables::BlockStatusses>(block_num)?;
            let status = res.ok_or(ProviderError::MissingBlockStatus(block_num))?;

            Ok(Some(status))
        } else {
            Ok(None)
        }
    }
}

impl<Tx: DbTx> TransactionTraceProvider for DbProvider<Tx> {
    fn transaction_execution(
        &self,
        hash: TxHash,
    ) -> ProviderResult<Option<TypedTransactionExecutionInfo>> {
        if let Some(num) = self.0.get::<tables::TxNumbers>(hash)? {
            let execution = self
                .0
                .get::<tables::TxTraces>(num)?
                .ok_or(ProviderError::MissingTxExecution(num))?;

            Ok(Some(execution))
        } else {
            Ok(None)
        }
    }

    fn transaction_executions_by_block(
        &self,
        block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<Vec<TypedTransactionExecutionInfo>>> {
        if let Some(index) = self.block_body_indices(block_id)? {
            let traces = self.transaction_executions_in_range(index.into())?;
            Ok(Some(traces))
        } else {
            Ok(None)
        }
    }

    fn transaction_executions_in_range(
        &self,
        range: Range<TxNumber>,
    ) -> ProviderResult<Vec<TypedTransactionExecutionInfo>> {
        let total = range.end - range.start;
        let mut traces = Vec::with_capacity(total as usize);

        for i in range {
            if let Some(trace) = self.0.get::<tables::TxTraces>(i)? {
                traces.push(trace);
            }
        }

        Ok(traces)
    }
}

impl<Tx: DbTx> ReceiptProvider for DbProvider<Tx> {
    fn receipt_by_hash(&self, hash: TxHash) -> ProviderResult<Option<Receipt>> {
        if let Some(num) = self.0.get::<tables::TxNumbers>(hash)? {
            let receipt =
                self.0.get::<tables::Receipts>(num)?.ok_or(ProviderError::MissingTxReceipt(num))?;

            Ok(Some(receipt))
        } else {
            Ok(None)
        }
    }

    fn receipts_by_block(
        &self,
        block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<Vec<Receipt>>> {
        if let Some(indices) = self.block_body_indices(block_id)? {
            let mut receipts = Vec::with_capacity(indices.tx_count as usize);

            let range = indices.tx_offset..indices.tx_offset + indices.tx_count;
            for i in range {
                if let Some(receipt) = self.0.get::<tables::Receipts>(i)? {
                    receipts.push(receipt);
                }
            }

            Ok(Some(receipts))
        } else {
            Ok(None)
        }
    }
}

impl<Tx: DbTx> BlockEnvProvider for DbProvider<Tx> {
    fn block_env_at(&self, block_id: BlockHashOrNumber) -> ProviderResult<Option<BlockEnv>> {
        let Some(header) = self.header(block_id)? else { return Ok(None) };

        Ok(Some(BlockEnv {
            number: header.number,
            timestamp: header.timestamp,
            l2_gas_prices: header.l2_gas_prices,
            l1_gas_prices: header.l1_gas_prices,
            l1_data_gas_prices: header.l1_data_gas_prices,
            sequencer_address: header.sequencer_address,
            starknet_version: header.starknet_version,
        }))
    }
}

impl<Tx: DbTxMut> BlockWriter for DbProvider<Tx> {
    fn insert_block_with_states_and_receipts(
        &self,
        block: SealedBlockWithStatus,
        states: StateUpdatesWithClasses,
        receipts: Vec<Receipt>,
        executions: Vec<TypedTransactionExecutionInfo>,
    ) -> ProviderResult<()> {
        let block_hash = block.block.hash;
        let block_number = block.block.header.number;

        let block_header = block.block.header;
        let transactions = block.block.body;

        let tx_count = transactions.len() as u64;
        let tx_offset = self.0.entries::<tables::Transactions>()? as u64;
        let block_body_indices = StoredBlockBodyIndices { tx_offset, tx_count };

        self.0.put::<tables::BlockHashes>(block_number, block_hash)?;
        self.0.put::<tables::BlockNumbers>(block_hash, block_number)?;
        self.0.put::<tables::BlockStatusses>(block_number, block.status)?;

        self.0.put::<tables::Headers>(block_number, VersionedHeader::from(block_header))?;
        self.0.put::<tables::BlockBodyIndices>(block_number, block_body_indices)?;

        // Store base transaction details
        for (i, transaction) in transactions.into_iter().enumerate() {
            let tx_number = tx_offset + i as u64;
            let tx_hash = transaction.hash;

            self.0.put::<tables::TxHashes>(tx_number, tx_hash)?;
            self.0.put::<tables::TxNumbers>(tx_hash, tx_number)?;
            self.0.put::<tables::TxBlocks>(tx_number, block_number)?;
            self.0.put::<tables::Transactions>(
                tx_number,
                VersionedTx::from(transaction.transaction),
            )?;
        }

        // Store transaction receipts
        for (i, receipt) in receipts.into_iter().enumerate() {
            let tx_number = tx_offset + i as u64;
            self.0.put::<tables::Receipts>(tx_number, receipt)?;
        }

        // Store execution traces
        for (i, execution) in executions.into_iter().enumerate() {
            let tx_number = tx_offset + i as u64;
            self.0.put::<tables::TxTraces>(tx_number, execution)?;
        }

        // insert all class artifacts
        for (class_hash, class) in states.classes {
            self.0.put::<tables::Classes>(class_hash, class.into())?;
        }

        // insert compiled class hashes and declarations for declared classes
        for (class_hash, compiled_hash) in states.state_updates.declared_classes {
            self.0.put::<tables::CompiledClassHashes>(class_hash, compiled_hash)?;
            self.0.put::<tables::ClassDeclarationBlock>(class_hash, block_number)?;
            self.0.put::<tables::ClassDeclarations>(block_number, class_hash)?;
        }

        // insert declarations for deprecated declared classes
        for class_hash in states.state_updates.deprecated_declared_classes {
            self.0.put::<tables::ClassDeclarationBlock>(class_hash, block_number)?;
            self.0.put::<tables::ClassDeclarations>(block_number, class_hash)?;
        }

        // insert migrated class hashes
        for (class_hash, compiled_class_hash) in states.state_updates.migrated_compiled_classes {
            let entry = MigratedCompiledClassHash { class_hash, compiled_class_hash };
            self.0.put::<tables::MigratedCompiledClassHashes>(block_number, entry)?;
        }

        // insert storage changes
        {
            let mut storage_cursor = self.0.cursor_dup_mut::<tables::ContractStorage>()?;
            for (addr, entries) in states.state_updates.storage_updates {
                let entries = entries.into_iter().map(|(key, value)| StorageEntry { key, value });

                for entry in entries {
                    match storage_cursor.seek_by_key_subkey(addr, entry.key)? {
                        Some(current) if current.key == entry.key => {
                            storage_cursor.delete_current()?;
                        }

                        _ => {}
                    }

                    // update block list in the change set
                    let changeset_key =
                        ContractStorageKey { contract_address: addr, key: entry.key };
                    let list = self.0.get::<tables::StorageChangeSet>(changeset_key.clone())?;

                    let updated_list = match list {
                        Some(mut list) => {
                            list.insert(block_number);
                            list
                        }
                        // create a new block list if it doesn't yet exist, and insert the block
                        // number
                        None => BlockList::from([block_number]),
                    };

                    self.0.put::<tables::StorageChangeSet>(changeset_key, updated_list)?;
                    storage_cursor.upsert(addr, entry)?;

                    let storage_change_sharded_key =
                        ContractStorageKey { contract_address: addr, key: entry.key };

                    self.0.put::<tables::StorageChangeHistory>(
                        block_number,
                        ContractStorageEntry {
                            key: storage_change_sharded_key,
                            value: entry.value,
                        },
                    )?;
                }
            }
        }

        // update contract info

        for (addr, class_hash) in states.state_updates.deployed_contracts {
            let value = if let Some(info) = self.0.get::<tables::ContractInfo>(addr)? {
                GenericContractInfo { class_hash, ..info }
            } else {
                GenericContractInfo { class_hash, ..Default::default() }
            };

            let new_change_set =
                if let Some(mut change_set) = self.0.get::<tables::ContractInfoChangeSet>(addr)? {
                    change_set.class_change_list.insert(block_number);
                    change_set
                } else {
                    ContractInfoChangeList {
                        class_change_list: BlockList::from([block_number]),
                        ..Default::default()
                    }
                };

            self.0.put::<tables::ContractInfo>(addr, value)?;

            let class_change_key = ContractClassChange::deployed(addr, class_hash);
            self.0.put::<tables::ClassChangeHistory>(block_number, class_change_key)?;
            self.0.put::<tables::ContractInfoChangeSet>(addr, new_change_set)?;
        }

        for (addr, new_class_hash) in states.state_updates.replaced_classes {
            let info = if let Some(info) = self.0.get::<tables::ContractInfo>(addr)? {
                GenericContractInfo { class_hash: new_class_hash, ..info }
            } else {
                GenericContractInfo { class_hash: new_class_hash, ..Default::default() }
            };

            let new_change_set =
                if let Some(mut change_set) = self.0.get::<tables::ContractInfoChangeSet>(addr)? {
                    change_set.class_change_list.insert(block_number);
                    change_set
                } else {
                    ContractInfoChangeList {
                        class_change_list: BlockList::from([block_number]),
                        ..Default::default()
                    }
                };

            self.0.put::<tables::ContractInfo>(addr, info)?;

            let class_change_key = ContractClassChange::replaced(addr, new_class_hash);
            self.0.put::<tables::ClassChangeHistory>(block_number, class_change_key)?;
            self.0.put::<tables::ContractInfoChangeSet>(addr, new_change_set)?;
        }

        for (addr, nonce) in states.state_updates.nonce_updates {
            let value = if let Some(info) = self.0.get::<tables::ContractInfo>(addr)? {
                GenericContractInfo { nonce, ..info }
            } else {
                GenericContractInfo { nonce, ..Default::default() }
            };

            let new_change_set =
                if let Some(mut change_set) = self.0.get::<tables::ContractInfoChangeSet>(addr)? {
                    change_set.nonce_change_list.insert(block_number);
                    change_set
                } else {
                    ContractInfoChangeList {
                        nonce_change_list: BlockList::from([block_number]),
                        ..Default::default()
                    }
                };

            self.0.put::<tables::ContractInfo>(addr, value)?;

            let nonce_change_key = ContractNonceChange { contract_address: addr, nonce };
            self.0.put::<tables::NonceChangeHistory>(block_number, nonce_change_key)?;
            self.0.put::<tables::ContractInfoChangeSet>(addr, new_change_set)?;
        }

        Ok(())
    }
}

impl<Tx: DbTxMut> StageCheckpointProvider for DbProvider<Tx> {
    fn execution_checkpoint(&self, id: &str) -> ProviderResult<Option<BlockNumber>> {
        let result = self.0.get::<tables::StageExecutionCheckpoints>(id.to_string())?;
        Ok(result.map(|x| x.block))
    }

    fn set_execution_checkpoint(&self, id: &str, block_number: BlockNumber) -> ProviderResult<()> {
        let key = id.to_string();
        let value = ExecutionCheckpoint { block: block_number };
        self.0.put::<tables::StageExecutionCheckpoints>(key, value)?;
        Ok(())
    }

    fn prune_checkpoint(&self, id: &str) -> ProviderResult<Option<BlockNumber>> {
        let result = self.0.get::<tables::StagePruningCheckpoints>(id.to_string())?;
        Ok(result.map(|x| x.block))
    }

    fn set_prune_checkpoint(&self, id: &str, block_number: BlockNumber) -> ProviderResult<()> {
        let key = id.to_string();
        let value = PruningCheckpoint { block: block_number };
        self.0.put::<tables::StagePruningCheckpoints>(key, value)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use katana_primitives::address;
    use katana_primitives::block::{
        Block, BlockHashOrNumber, FinalityStatus, Header, SealedBlockWithStatus,
    };
    use katana_primitives::class::ContractClass;
    use katana_primitives::execution::TypedTransactionExecutionInfo;
    use katana_primitives::fee::FeeInfo;
    use katana_primitives::receipt::{InvokeTxReceipt, Receipt};
    use katana_primitives::state::{StateUpdates, StateUpdatesWithClasses};
    use katana_primitives::transaction::{InvokeTx, Tx, TxHash, TxWithHash};
    use katana_provider_api::block::{
        BlockHashProvider, BlockNumberProvider, BlockProvider, BlockStatusProvider, BlockWriter,
    };
    use katana_provider_api::state::StateFactoryProvider;
    use katana_provider_api::transaction::TransactionProvider;
    use starknet::macros::felt;

    use crate::{DbProviderFactory, ProviderFactory};

    fn create_dummy_block() -> SealedBlockWithStatus {
        let header = Header { parent_hash: 199u8.into(), number: 0, ..Default::default() };
        let block = Block {
            header,
            body: vec![TxWithHash {
                hash: 24u8.into(),
                transaction: Tx::Invoke(InvokeTx::V1(Default::default())),
            }],
        }
        .seal();
        SealedBlockWithStatus { block, status: FinalityStatus::AcceptedOnL2 }
    }

    fn create_dummy_state_updates() -> StateUpdatesWithClasses {
        StateUpdatesWithClasses {
            state_updates: StateUpdates {
                nonce_updates: BTreeMap::from([
                    (address!("1"), felt!("1")),
                    (address!("2"), felt!("2")),
                ]),
                deployed_contracts: BTreeMap::from([
                    (address!("1"), felt!("3")),
                    (address!("2"), felt!("4")),
                ]),
                declared_classes: BTreeMap::from([
                    (felt!("3"), felt!("89")),
                    (felt!("4"), felt!("90")),
                ]),
                storage_updates: BTreeMap::from([(
                    address!("1"),
                    BTreeMap::from([(felt!("1"), felt!("1")), (felt!("2"), felt!("2"))]),
                )]),
                ..Default::default()
            },
            classes: BTreeMap::from([
                (felt!("3"), ContractClass::Legacy(Default::default())),
                (felt!("4"), ContractClass::Legacy(Default::default())),
            ]),
            ..Default::default()
        }
    }

    fn create_dummy_state_updates_2() -> StateUpdatesWithClasses {
        StateUpdatesWithClasses {
            state_updates: StateUpdates {
                nonce_updates: BTreeMap::from([
                    (address!("1"), felt!("5")),
                    (address!("2"), felt!("6")),
                ]),
                deployed_contracts: BTreeMap::from([
                    (address!("1"), felt!("77")),
                    (address!("2"), felt!("66")),
                ]),
                storage_updates: BTreeMap::from([(
                    address!("1"),
                    BTreeMap::from([(felt!("1"), felt!("100")), (felt!("2"), felt!("200"))]),
                )]),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn create_db_provider() -> DbProviderFactory {
        DbProviderFactory::new_in_memory()
    }

    #[test]
    fn insert_block() {
        let provider = create_db_provider();
        let provider = provider.provider_mut();
        let block = create_dummy_block();
        let state_updates = create_dummy_state_updates();

        // insert block
        provider
            .insert_block_with_states_and_receipts(
                block.clone(),
                state_updates,
                vec![Receipt::Invoke(InvokeTxReceipt {
                    revert_error: None,
                    events: Vec::new(),
                    messages_sent: Vec::new(),
                    fee: FeeInfo::default(),
                    execution_resources: Default::default(),
                })],
                vec![TypedTransactionExecutionInfo::default()],
            )
            .expect("failed to insert block");

        // get values

        let block_id: BlockHashOrNumber = block.block.hash.into();

        let latest_number = provider.latest_number().unwrap();
        let latest_hash = provider.latest_hash().unwrap();

        let actual_block = provider.block(block_id).unwrap().unwrap();
        let tx_count = provider.transaction_count_by_block(block_id).unwrap().unwrap();
        let block_status = provider.block_status(block_id).unwrap().unwrap();
        let body_indices = provider.block_body_indices(block_id).unwrap().unwrap();

        let tx_hash: TxHash = 24u8.into();
        let tx = provider.transaction_by_hash(tx_hash).unwrap().unwrap();

        let state_prov = provider.latest().unwrap();

        let nonce1 = state_prov.nonce(address!("1")).unwrap().unwrap();
        let nonce2 = state_prov.nonce(address!("2")).unwrap().unwrap();

        let class_hash1 = state_prov.class_hash_of_contract(felt!("1").into()).unwrap().unwrap();
        let class_hash2 = state_prov.class_hash_of_contract(felt!("2").into()).unwrap().unwrap();

        let compiled_hash1 =
            state_prov.compiled_class_hash_of_class_hash(class_hash1).unwrap().unwrap();
        let compiled_hash2 =
            state_prov.compiled_class_hash_of_class_hash(class_hash2).unwrap().unwrap();

        let storage1 = state_prov.storage(address!("1"), felt!("1")).unwrap().unwrap();
        let storage2 = state_prov.storage(address!("1"), felt!("2")).unwrap().unwrap();

        // assert values are populated correctly

        assert_eq!(tx_hash, tx.hash);
        assert_eq!(tx.transaction, Tx::Invoke(InvokeTx::V1(Default::default())));

        assert_eq!(tx_count, 1);
        assert_eq!(body_indices.tx_offset, 0);
        assert_eq!(body_indices.tx_count, tx_count);

        assert_eq!(block_status, FinalityStatus::AcceptedOnL2);
        assert_eq!(block.block.hash, latest_hash);
        assert_eq!(block.block.body.len() as u64, tx_count);
        assert_eq!(block.block.header.number, latest_number);
        assert_eq!(block.block.unseal(), actual_block);

        assert_eq!(nonce1, felt!("1"));
        assert_eq!(nonce2, felt!("2"));
        assert_eq!(class_hash1, felt!("3"));
        assert_eq!(class_hash2, felt!("4"));

        assert_eq!(compiled_hash1, felt!("89"));
        assert_eq!(compiled_hash2, felt!("90"));

        assert_eq!(storage1, felt!("1"));
        assert_eq!(storage2, felt!("2"));
    }

    #[test]
    fn storage_updated_correctly() {
        let provider = create_db_provider();
        let provider = provider.provider_mut();

        let block = create_dummy_block();
        let state_updates1 = create_dummy_state_updates();
        let state_updates2 = create_dummy_state_updates_2();

        // insert block
        provider
            .insert_block_with_states_and_receipts(
                block.clone(),
                state_updates1,
                vec![Receipt::Invoke(InvokeTxReceipt {
                    revert_error: None,
                    events: Vec::new(),
                    messages_sent: Vec::new(),
                    fee: FeeInfo::default(),
                    execution_resources: Default::default(),
                })],
                vec![TypedTransactionExecutionInfo::default()],
            )
            .expect("failed to insert block");

        // insert another block
        provider
            .insert_block_with_states_and_receipts(
                block,
                state_updates2,
                vec![Receipt::Invoke(InvokeTxReceipt {
                    revert_error: None,
                    events: Vec::new(),
                    messages_sent: Vec::new(),
                    fee: FeeInfo::default(),
                    execution_resources: Default::default(),
                })],
                vec![TypedTransactionExecutionInfo::default()],
            )
            .expect("failed to insert block");

        // assert storage is updated correctly

        let state_prov = StateFactoryProvider::latest(&provider).unwrap();

        let nonce1 = state_prov.nonce(address!("1")).unwrap().unwrap();
        let nonce2 = state_prov.nonce(address!("2")).unwrap().unwrap();

        let class_hash1 = state_prov.class_hash_of_contract(felt!("1").into()).unwrap().unwrap();
        let class_hash2 = state_prov.class_hash_of_contract(felt!("2").into()).unwrap().unwrap();

        let storage1 = state_prov.storage(address!("1"), felt!("1")).unwrap().unwrap();
        let storage2 = state_prov.storage(address!("1"), felt!("2")).unwrap().unwrap();

        assert_eq!(nonce1, felt!("5"));
        assert_eq!(nonce2, felt!("6"));

        assert_eq!(class_hash1, felt!("77"));
        assert_eq!(class_hash2, felt!("66"));

        assert_eq!(storage1, felt!("100"));
        assert_eq!(storage2, felt!("200"));
    }
}
