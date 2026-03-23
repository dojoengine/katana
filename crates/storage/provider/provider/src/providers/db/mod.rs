pub mod state;
pub mod trie;

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::ops::{Deref, Range, RangeInclusive};

use katana_db::abstraction::{DbCursor, DbCursorMut, DbDupSortCursor, DbTx, DbTxMut};
use katana_db::error::{CodecError, DatabaseError};
use katana_db::models::block::StoredBlockBodyIndices;
use katana_db::models::class::MigratedCompiledClassHash;
use katana_db::models::contract::{
    ContractClassChange, ContractInfoChangeList, ContractNonceChange,
};
use katana_db::models::list::BlockChangeList;
use katana_db::models::stage::{ExecutionCheckpoint, PruningCheckpoint};
use katana_db::models::state::HistoricalStateRetention;
use katana_db::models::storage::{ContractStorageEntry, ContractStorageKey, StorageEntry};
use katana_db::models::{
    ReceiptEnvelope, StateUpdateEnvelope, TxEnvelope, VersionedHeader, VersionedTx,
};
use katana_db::tables;
use katana_primitives::block::{
    Block, BlockHash, BlockHashOrNumber, BlockNumber, BlockWithTxHashes, FinalityStatus, Header,
    SealedBlockWithStatus,
};
use katana_primitives::class::{ClassHash, CompiledClassHash};
use katana_primitives::contract::{ContractAddress, GenericContractInfo};
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
use katana_provider_api::state::HistoricalStateRetentionProvider;
use katana_provider_api::state_update::StateUpdateProvider;
use katana_provider_api::transaction::{
    ReceiptProvider, TransactionProvider, TransactionStatusProvider, TransactionTraceProvider,
    TransactionsProviderExt,
};
use katana_provider_api::ProviderError;
use katana_rpc_types::{TxTrace, TxTraceWithHash};
use tracing::warn;

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

    fn canonical_state_update_by_number(
        &self,
        block_number: BlockNumber,
    ) -> ProviderResult<StateUpdates> {
        let envelope = self
            .0
            .get::<tables::BlockStateUpdates>(block_number)?
            .ok_or(ProviderError::MissingBlockStateUpdate(block_number))?;

        Ok(StateUpdates::from(envelope))
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

impl<Tx: DbTx> StateUpdateProvider for DbProvider<Tx> {
    fn state_update(&self, block_id: BlockHashOrNumber) -> ProviderResult<Option<StateUpdates>> {
        let block_num = self.block_number_by_id(block_id)?;

        if let Some(block_num) = block_num {
            Ok(Some(self.canonical_state_update_by_number(block_num)?))
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
            Ok(Some(self.canonical_state_update_by_number(block_num)?.declared_classes))
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
            Ok(Some(self.canonical_state_update_by_number(block_num)?.deployed_contracts))
        } else {
            Ok(None)
        }
    }
}

impl<Tx: DbTx> TransactionProvider for DbProvider<Tx> {
    fn transaction_by_hash(&self, hash: TxHash) -> ProviderResult<Option<TxWithHash>> {
        if let Some(num) = self.0.get::<tables::TxNumbers>(hash)? {
            let res = self.0.get::<tables::Transactions>(num)?;
            let envelope = res.ok_or(ProviderError::MissingTx(num))?;
            Ok(Some(TxWithHash { hash, transaction: envelope.inner.into() }))
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
            if let Some(envelope) = self.0.get::<tables::Transactions>(i)? {
                let res = self.0.get::<tables::TxHashes>(i)?;
                let hash = res.ok_or(ProviderError::MissingTxHash(i))?;
                transactions.push(TxWithHash { hash, transaction: envelope.inner.into() });
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
                let envelope = res.ok_or(ProviderError::MissingTx(num))?;

                Ok(Some(TxWithHash { hash, transaction: envelope.inner.into() }))
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

/// NOTE:
///
/// The `TransactionExecutionInfo` type (from the `blockifier` crate) has had breaking
/// serialization changes between versions. Entries stored with older versions may fail to
/// deserialize.
///
/// Though this may change in the future, this behavior is currently necessary to maintain
/// backward compatibility. As a compromise, traces that cannot be deserialized
/// are treated as non-existent rather than causing errors.
impl<Tx: DbTx> TransactionTraceProvider for DbProvider<Tx> {
    fn transaction_execution(
        &self,
        hash: TxHash,
    ) -> ProviderResult<Option<TypedTransactionExecutionInfo>> {
        if let Some(num) = self.0.get::<tables::TxNumbers>(hash)? {
            match self.0.get::<tables::TxTraces>(num) {
                Ok(Some(execution)) => Ok(Some(execution)),
                Ok(None) => Ok(None),
                // Treat decompress errors as non-existent for backward compatibility
                Err(DatabaseError::Codec(CodecError::Decompress(err))) => {
                    warn!(tx_num = %num, %err, "Failed to deserialize transaction trace");
                    Ok(None)
                }
                Err(e) => Err(e.into()),
            }
        } else {
            Ok(None)
        }
    }

    fn transaction_executions_by_block(
        &self,
        block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<Vec<TxTraceWithHash>>> {
        if let Some(index) = self.block_body_indices(block_id)? {
            let traces = self.transaction_executions_in_range(index.clone().into())?;
            let traces = traces.into_iter().map(TxTrace::from);

            let tx_hashes = self.transaction_hashes_in_range(index.into())?;

            let traces = tx_hashes
                .into_iter()
                .zip(traces)
                .map(|(h, r)| TxTraceWithHash { transaction_hash: h, trace_root: r })
                .collect::<Vec<_>>();

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
            match self.0.get::<tables::TxTraces>(i) {
                Ok(Some(trace)) => traces.push(trace),
                Ok(None) => {}
                // Skip entries that fail to decompress for backward compatibility
                Err(DatabaseError::Codec(CodecError::Decompress(err))) => {
                    warn!(tx_num = %i, %err, "Failed to deserialize transaction trace");
                }
                Err(e) => return Err(e.into()),
            }
        }

        Ok(traces)
    }
}

impl<Tx: DbTx> ReceiptProvider for DbProvider<Tx> {
    fn receipt_by_hash(&self, hash: TxHash) -> ProviderResult<Option<Receipt>> {
        if let Some(num) = self.0.get::<tables::TxNumbers>(hash)? {
            let receipt = self
                .0
                .get::<tables::Receipts>(num)?
                .ok_or(ProviderError::MissingTxReceipt(num))
                .map(Receipt::from)?;

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
                    receipts.push(receipt.into());
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

impl<Tx: DbTxMut> DbProvider<Tx> {
    /// Stores block data without building historical state indices.
    ///
    /// This stores: headers, hashes, body indices, `BlockStateUpdates`, txs, receipts, traces,
    /// class artifacts, compiled class hashes, class declarations, deprecated declarations,
    /// migrated compiled class hashes.
    pub fn insert_block_data(
        &self,
        block: SealedBlockWithStatus,
        states: StateUpdatesWithClasses,
        receipts: Vec<Receipt>,
        executions: Vec<TypedTransactionExecutionInfo>,
    ) -> ProviderResult<()> {
        let block_hash = block.block.hash;
        let block_number = block.block.header.number;
        let StateUpdatesWithClasses { state_updates, classes } = states;

        let block_header = block.block.header;
        let transactions = block.block.body;

        let tx_count = transactions.len() as u64;
        let tx_offset = self.0.entries::<tables::Transactions>()? as u64;
        let block_body_indices = StoredBlockBodyIndices { tx_offset, tx_count };

        self.0.put::<tables::BlockHashes>(block_number, block_hash)?;
        self.0.put::<tables::BlockNumbers>(block_hash, block_number)?;
        self.0.put::<tables::BlockStatusses>(block_number, block.status)?;

        self.0.put::<tables::Headers>(block_number, VersionedHeader::from(block_header))?;
        self.0.put::<tables::BlockStateUpdates>(
            block_number,
            StateUpdateEnvelope::from(state_updates.clone()),
        )?;
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
                TxEnvelope::from(VersionedTx::from(transaction.transaction)),
            )?;
        }

        // Store transaction receipts
        for (i, receipt) in receipts.into_iter().enumerate() {
            let tx_number = tx_offset + i as u64;
            // `Receipts` table stores a dedicated envelope so storage format can evolve without
            // changing the in-memory `Receipt` type codec.
            self.0.put::<tables::Receipts>(tx_number, ReceiptEnvelope::from(receipt))?;
        }

        // Store execution traces
        for (i, execution) in executions.into_iter().enumerate() {
            let tx_number = tx_offset + i as u64;
            self.0.put::<tables::TxTraces>(tx_number, execution)?;
        }

        // insert all class artifacts
        for (class_hash, class) in classes {
            self.0.put::<tables::Classes>(class_hash, class.into())?;
        }

        // insert compiled class hashes and declarations for declared classes
        for (class_hash, compiled_hash) in state_updates.declared_classes {
            self.0.put::<tables::CompiledClassHashes>(class_hash, compiled_hash)?;
            self.0.put::<tables::ClassDeclarationBlock>(class_hash, block_number)?;
            self.0.put::<tables::ClassDeclarations>(block_number, class_hash)?;
        }

        // insert declarations for deprecated declared classes
        for class_hash in state_updates.deprecated_declared_classes {
            self.0.put::<tables::ClassDeclarationBlock>(class_hash, block_number)?;
            self.0.put::<tables::ClassDeclarations>(block_number, class_hash)?;
        }

        // insert migrated class hashes
        for (class_hash, compiled_class_hash) in state_updates.migrated_compiled_classes {
            let entry = MigratedCompiledClassHash { class_hash, compiled_class_hash };
            self.0.put::<tables::MigratedCompiledClassHashes>(block_number, entry)?;
        }

        Ok(())
    }

    /// Builds historical state indices for a range of blocks in bulk.
    ///
    /// This is an optimized path for first sync (when the history tables are empty). Instead of
    /// doing read-modify-write per block, it accumulates all changes in memory and writes each
    /// key once at the end. The DupSort history tables (`StorageChangeHistory`,
    /// `NonceChangeHistory`, `ClassChangeHistory`) are written per-block since blocks are
    /// processed in order and the keys are monotonically increasing.
    ///
    /// Updates the same tables as [`insert_state_history`](Self::insert_state_history).
    pub fn insert_state_history_bulk(
        &self,
        blocks: impl IntoIterator<Item = (BlockNumber, StateUpdates)>,
    ) -> ProviderResult<()> {
        // Accumulated state: written once per key at the end.
        let mut storage_change_sets: BTreeMap<ContractStorageKey, BlockChangeList> =
            BTreeMap::new();
        let mut contract_info_change_sets: BTreeMap<ContractAddress, ContractInfoChangeList> =
            BTreeMap::new();
        // Track latest storage value per (addr, key) — only the final value is written.
        let mut latest_storage: BTreeMap<(ContractAddress, katana_primitives::Felt), StorageEntry> =
            BTreeMap::new();
        // Track latest contract info per address — only the final state is written.
        let mut latest_contract_info: BTreeMap<ContractAddress, GenericContractInfo> =
            BTreeMap::new();

        for (block_number, state_updates) in blocks {
            // -- storage changes --
            for (addr, entries) in &state_updates.storage_updates {
                for (key, value) in entries {
                    let entry = StorageEntry { key: *key, value: *value };

                    // Accumulate change set
                    let changeset_key = ContractStorageKey { contract_address: *addr, key: *key };
                    storage_change_sets
                        .entry(changeset_key.clone())
                        .or_default()
                        .insert(block_number);

                    // Track latest value
                    latest_storage.insert((*addr, *key), entry);

                    // Write per-block history entry (block-keyed DupSort, sequential)
                    self.0.put::<tables::StorageChangeHistory>(
                        block_number,
                        ContractStorageEntry { key: changeset_key, value: *value },
                    )?;
                }
            }

            // -- deployed contracts --
            for (addr, class_hash) in &state_updates.deployed_contracts {
                let info = latest_contract_info.entry(*addr).or_default();
                info.class_hash = *class_hash;

                contract_info_change_sets
                    .entry(*addr)
                    .or_default()
                    .class_change_list
                    .insert(block_number);

                let class_change_key = ContractClassChange::deployed(*addr, *class_hash);
                self.0.put::<tables::ClassChangeHistory>(block_number, class_change_key)?;
            }

            // -- replaced classes --
            for (addr, new_class_hash) in &state_updates.replaced_classes {
                let info = latest_contract_info.entry(*addr).or_default();
                info.class_hash = *new_class_hash;

                contract_info_change_sets
                    .entry(*addr)
                    .or_default()
                    .class_change_list
                    .insert(block_number);

                let class_change_key = ContractClassChange::replaced(*addr, *new_class_hash);
                self.0.put::<tables::ClassChangeHistory>(block_number, class_change_key)?;
            }

            // -- nonce updates --
            for (addr, nonce) in &state_updates.nonce_updates {
                let info = latest_contract_info.entry(*addr).or_default();
                info.nonce = *nonce;

                contract_info_change_sets
                    .entry(*addr)
                    .or_default()
                    .nonce_change_list
                    .insert(block_number);

                let nonce_change_key =
                    ContractNonceChange { contract_address: *addr, nonce: *nonce };
                self.0.put::<tables::NonceChangeHistory>(block_number, nonce_change_key)?;
            }
        }

        // Flush accumulated storage change sets (one write per key).
        for (key, block_list) in storage_change_sets {
            self.0.put::<tables::StorageChangeSet>(key, block_list)?;
        }

        // Flush latest storage values (one write per (addr, key)).
        {
            let mut storage_cursor = self.0.cursor_dup_mut::<tables::ContractStorage>()?;
            for ((addr, _), entry) in &latest_storage {
                storage_cursor.upsert(*addr, *entry)?;
            }
        }

        // Flush accumulated contract info change sets (one write per address).
        for (addr, change_set) in contract_info_change_sets {
            self.0.put::<tables::ContractInfoChangeSet>(addr, change_set)?;
        }

        // Flush latest contract info (one write per address).
        for (addr, info) in latest_contract_info {
            self.0.put::<tables::ContractInfo>(addr, info)?;
        }

        Ok(())
    }

    /// Builds historical state indices for a single block from its state updates.
    ///
    /// This updates: `ContractStorage`, `StorageChangeSet`, `StorageChangeHistory`,
    /// `ContractInfo`, `ContractInfoChangeSet`, `ClassChangeHistory`, `NonceChangeHistory`.
    pub fn insert_state_history(
        &self,
        block_number: BlockNumber,
        state_updates: &StateUpdates,
    ) -> ProviderResult<()> {
        // insert storage changes
        {
            let mut storage_cursor = self.0.cursor_dup_mut::<tables::ContractStorage>()?;
            for (addr, entries) in &state_updates.storage_updates {
                let entries =
                    entries.iter().map(|(key, value)| StorageEntry { key: *key, value: *value });

                for entry in entries {
                    match storage_cursor.seek_by_key_subkey(*addr, entry.key)? {
                        Some(current) if current.key == entry.key => {
                            storage_cursor.delete_current()?;
                        }

                        _ => {}
                    }

                    // update block list in the change set
                    let changeset_key =
                        ContractStorageKey { contract_address: *addr, key: entry.key };
                    let list = self.0.get::<tables::StorageChangeSet>(changeset_key.clone())?;

                    let updated_list = match list {
                        Some(mut list) => {
                            list.insert(block_number);
                            list
                        }
                        // create a new block list if it doesn't yet exist, and insert the block
                        // number
                        None => BlockChangeList::from([block_number]),
                    };

                    self.0.put::<tables::StorageChangeSet>(changeset_key, updated_list)?;
                    storage_cursor.upsert(*addr, entry)?;

                    let storage_change_sharded_key =
                        ContractStorageKey { contract_address: *addr, key: entry.key };

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

        for (addr, class_hash) in &state_updates.deployed_contracts {
            let value = if let Some(info) = self.0.get::<tables::ContractInfo>(*addr)? {
                GenericContractInfo { class_hash: *class_hash, ..info }
            } else {
                GenericContractInfo { class_hash: *class_hash, ..Default::default() }
            };

            let new_change_set =
                if let Some(mut change_set) = self.0.get::<tables::ContractInfoChangeSet>(*addr)? {
                    change_set.class_change_list.insert(block_number);
                    change_set
                } else {
                    ContractInfoChangeList {
                        class_change_list: BlockChangeList::from([block_number]),
                        ..Default::default()
                    }
                };

            self.0.put::<tables::ContractInfo>(*addr, value)?;

            let class_change_key = ContractClassChange::deployed(*addr, *class_hash);
            self.0.put::<tables::ClassChangeHistory>(block_number, class_change_key)?;
            self.0.put::<tables::ContractInfoChangeSet>(*addr, new_change_set)?;
        }

        for (addr, new_class_hash) in &state_updates.replaced_classes {
            let info = if let Some(info) = self.0.get::<tables::ContractInfo>(*addr)? {
                GenericContractInfo { class_hash: *new_class_hash, ..info }
            } else {
                GenericContractInfo { class_hash: *new_class_hash, ..Default::default() }
            };

            let new_change_set =
                if let Some(mut change_set) = self.0.get::<tables::ContractInfoChangeSet>(*addr)? {
                    change_set.class_change_list.insert(block_number);
                    change_set
                } else {
                    ContractInfoChangeList {
                        class_change_list: BlockChangeList::from([block_number]),
                        ..Default::default()
                    }
                };

            self.0.put::<tables::ContractInfo>(*addr, info)?;

            let class_change_key = ContractClassChange::replaced(*addr, *new_class_hash);
            self.0.put::<tables::ClassChangeHistory>(block_number, class_change_key)?;
            self.0.put::<tables::ContractInfoChangeSet>(*addr, new_change_set)?;
        }

        for (addr, nonce) in &state_updates.nonce_updates {
            let value = if let Some(info) = self.0.get::<tables::ContractInfo>(*addr)? {
                GenericContractInfo { nonce: *nonce, ..info }
            } else {
                GenericContractInfo { nonce: *nonce, ..Default::default() }
            };

            let new_change_set =
                if let Some(mut change_set) = self.0.get::<tables::ContractInfoChangeSet>(*addr)? {
                    change_set.nonce_change_list.insert(block_number);
                    change_set
                } else {
                    ContractInfoChangeList {
                        nonce_change_list: BlockChangeList::from([block_number]),
                        ..Default::default()
                    }
                };

            self.0.put::<tables::ContractInfo>(*addr, value)?;

            let nonce_change_key = ContractNonceChange { contract_address: *addr, nonce: *nonce };
            self.0.put::<tables::NonceChangeHistory>(block_number, nonce_change_key)?;
            self.0.put::<tables::ContractInfoChangeSet>(*addr, new_change_set)?;
        }

        Ok(())
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
        let block_number = block.block.header.number;
        let state_updates = states.state_updates.clone();
        self.insert_block_data(block, states, receipts, executions)?;
        self.insert_state_history(block_number, &state_updates)?;
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

pub const STATE_HISTORY_RETENTION_KEY: u64 = 0;
pub const STATE_TRIE_HISTORY_RETENTION_KEY: u64 = 1;

impl<Tx: DbTxMut> HistoricalStateRetentionProvider for DbProvider<Tx> {
    fn earliest_available_state_block(&self) -> ProviderResult<Option<BlockNumber>> {
        let key = STATE_HISTORY_RETENTION_KEY;
        if let Some(retention) = self.0.get::<tables::StateHistoryRetention>(key)? {
            return Ok(Some(retention.earliest_available_block));
        }
        // No pruning entry — return block 0 if any blocks exist, None otherwise.
        let has_blocks = self.0.cursor::<tables::BlockHashes>()?.last()?.is_some();
        Ok(has_blocks.then_some(0))
    }

    fn set_earliest_available_state_block(&self, block_number: BlockNumber) -> ProviderResult<()> {
        let key = STATE_HISTORY_RETENTION_KEY;
        let value = HistoricalStateRetention { earliest_available_block: block_number };
        self.0.put::<tables::StateHistoryRetention>(key, value)?;
        Ok(())
    }

    fn earliest_available_state_trie_block(&self) -> ProviderResult<Option<BlockNumber>> {
        let key = STATE_TRIE_HISTORY_RETENTION_KEY;
        if let Some(retention) = self.0.get::<tables::StateHistoryRetention>(key)? {
            return Ok(Some(retention.earliest_available_block));
        }
        // No pruning entry — return block 0 if any blocks exist, None otherwise.
        let has_blocks = self.0.cursor::<tables::BlockHashes>()?.last()?.is_some();
        Ok(has_blocks.then_some(0))
    }

    fn set_earliest_available_state_trie_block(
        &self,
        block_number: BlockNumber,
    ) -> ProviderResult<()> {
        let key = STATE_TRIE_HISTORY_RETENTION_KEY;
        let value = HistoricalStateRetention { earliest_available_block: block_number };
        self.0.put::<tables::StateHistoryRetention>(key, value)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use katana_primitives::block::{
        Block, BlockHashOrNumber, FinalityStatus, Header, SealedBlockWithStatus,
    };
    use katana_primitives::class::ContractClass;
    use katana_primitives::execution::TypedTransactionExecutionInfo;
    use katana_primitives::fee::FeeInfo;
    use katana_primitives::receipt::{InvokeTxReceipt, Receipt};
    use katana_primitives::state::{StateUpdates, StateUpdatesWithClasses};
    use katana_primitives::transaction::{InvokeTx, Tx, TxHash, TxWithHash};
    use katana_primitives::{address, felt};
    use katana_provider_api::block::{
        BlockHashProvider, BlockNumberProvider, BlockProvider, BlockStatusProvider, BlockWriter,
    };
    use katana_provider_api::state::StateFactoryProvider;
    use katana_provider_api::transaction::TransactionProvider;

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
