pub mod state;
pub mod trie;

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::ops::{Deref, Range, RangeInclusive};
use std::sync::Arc;

use katana_db::abstraction::{DbCursor, DbCursorMut, DbDupSortCursor, DbTx, DbTxMut};
use katana_db::codecs::{Compress, Decompress};
use katana_db::error::CodecError;
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
    ReceiptEnvelope, StateUpdateEnvelope, StaticFileRef, TxEnvelope, VersionedHeader, VersionedTx,
};
use katana_db::static_files::segment::StaticFileError;
use katana_db::static_files::{AnyStore, StaticFiles};
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
use tracing::warn;

use crate::{MutableProvider, ProviderResult};

/// Resolve a [`StaticFileRef`] to a typed value.
///
/// - `StaticFile { offset, length }` -> reads from the static file via `read_fn`, then
///   decompresses.
/// - `Inline(bytes)` -> decompresses directly from the MDBX-stored bytes.
///
/// The inline path is used in fork mode where static files are not written.
pub(crate) fn resolve_static_ref<T: Decompress>(
    static_files: &StaticFiles<AnyStore>,
    sf_ref: &StaticFileRef,
    read_fn: impl FnOnce(&StaticFiles<AnyStore>, u64, u32) -> Result<T, StaticFileError>,
) -> ProviderResult<T> {
    match sf_ref {
        StaticFileRef::StaticFile { offset, length } => {
            read_fn(static_files, *offset, *length).map_err(ProviderError::StaticFile)
        }
        StaticFileRef::Inline(data) => {
            T::decompress(data).map_err(|e| ProviderError::Other(e.to_string()))
        }
    }
}

/// Compress a value for inline storage in MDBX (fork mode).
fn compress_value<T: Compress>(value: T) -> ProviderResult<Vec<u8>> {
    Ok(value.compress().map_err(|e| ProviderError::Other(e.to_string()))?.into())
}

/// A provider implementation that uses a persistent database as the backend.
#[derive(Clone)]
pub struct DbProvider<Tx: DbTx> {
    tx: Tx,
    static_files: Arc<StaticFiles<AnyStore>>,
}

impl<Tx: DbTx> Deref for DbProvider<Tx> {
    type Target = Tx;

    fn deref(&self) -> &Self::Target {
        &self.tx
    }
}

impl<Tx: DbTx + Debug> Debug for DbProvider<Tx> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DbProvider").field("tx", &self.tx).finish_non_exhaustive()
    }
}

impl<Tx: DbTx> DbProvider<Tx> {
    /// Creates a new [`DbProvider`] from the given transaction and static files.
    pub fn new(tx: Tx, static_files: Arc<StaticFiles<AnyStore>>) -> Self {
        Self { tx, static_files }
    }

    /// Returns the [`DbTx`] associated with this provider.
    pub fn tx(&self) -> &Tx {
        &self.tx
    }

    /// Returns a reference to the static files storage.
    pub fn static_files(&self) -> &Arc<StaticFiles<AnyStore>> {
        &self.static_files
    }

    /// Read tx hash: try static files first, fall back to MDBX.
    fn get_tx_hash(&self, num: TxNumber) -> ProviderResult<Option<TxHash>> {
        if let Some(hash) =
            self.static_files.read_tx_hash(num).map_err(ProviderError::StaticFile)?
        {
            return Ok(Some(hash));
        }
        Ok(self.tx.get::<tables::TxHashes>(num)?)
    }

    /// Read tx block number: try static files first, fall back to MDBX.
    fn get_tx_block(&self, num: TxNumber) -> ProviderResult<Option<BlockNumber>> {
        if let Some(block) =
            self.static_files.read_tx_block(num).map_err(ProviderError::StaticFile)?
        {
            return Ok(Some(block));
        }
        Ok(self.tx.get::<tables::TxBlocks>(num)?)
    }

    fn canonical_state_update_by_number(
        &self,
        block_number: BlockNumber,
    ) -> ProviderResult<StateUpdates> {
        let sf_ref = self
            .tx
            .get::<tables::BlockStateUpdates>(block_number)?
            .ok_or(ProviderError::MissingBlockStateUpdate(block_number))?;

        let envelope: StateUpdateEnvelope =
            resolve_static_ref(&self.static_files, &sf_ref, |sf, o, l| {
                sf.read_block_state_update(o, l)
            })?;

        Ok(StateUpdates::from(envelope))
    }
}

impl<Tx: DbTxMut> MutableProvider for DbProvider<Tx> {
    fn commit(self) -> ProviderResult<()> {
        // Static files are NOT fsynced here. On crash, MDBX state determines what
        // exists, and orphaned static file data is truncated on next startup.
        // This matches MDBX's own durability model (the caller controls sync mode).
        let _ = self.tx.commit()?;
        // Refresh mmap views so subsequent readers see the newly-written data.
        // This is lightweight (no I/O) — just updates the mmap pointers.
        let _ = self.static_files.remap();
        Ok(())
    }
}

impl<Tx: DbTx> BlockNumberProvider for DbProvider<Tx> {
    fn block_number_by_hash(&self, hash: BlockHash) -> ProviderResult<Option<BlockNumber>> {
        let block_num = self.tx.get::<tables::BlockNumbers>(hash)?;
        Ok(block_num)
    }

    fn latest_number(&self) -> ProviderResult<BlockNumber> {
        let res = self.tx.cursor::<tables::Headers>()?.last()?.map(|(num, _)| num);
        res.ok_or(ProviderError::MissingLatestBlockNumber)
    }
}

impl<Tx: DbTx> BlockIdReader for DbProvider<Tx> {}

impl<Tx: DbTx> BlockHashProvider for DbProvider<Tx> {
    fn latest_hash(&self) -> ProviderResult<BlockHash> {
        let latest = self.latest_number()?;
        self.block_hash_by_num(latest)?.ok_or(ProviderError::MissingLatestBlockHash)
    }

    fn block_hash_by_num(&self, num: BlockNumber) -> ProviderResult<Option<BlockHash>> {
        // Try static files first (sequential mode), fall back to MDBX (fork mode).
        if let Some(hash) =
            self.static_files.read_block_hash(num).map_err(ProviderError::StaticFile)?
        {
            return Ok(Some(hash));
        }
        Ok(self.tx.get::<tables::BlockHashes>(num)?)
    }
}

impl<Tx: DbTx> HeaderProvider for DbProvider<Tx> {
    fn header(&self, id: BlockHashOrNumber) -> ProviderResult<Option<Header>> {
        let num = match id {
            BlockHashOrNumber::Num(num) => Some(num),
            BlockHashOrNumber::Hash(hash) => self.tx.get::<tables::BlockNumbers>(hash)?,
        };

        let Some(num) = num else { return Ok(None) };

        let sf_ref = self.tx.get::<tables::Headers>(num)?;
        match sf_ref {
            Some(r) => {
                let header: VersionedHeader =
                    resolve_static_ref(&self.static_files, &r, |sf, o, l| sf.read_header(o, l))?;
                Ok(Some(header.into()))
            }
            None => Ok(None),
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
            BlockHashOrNumber::Hash(hash) => self.tx.get::<tables::BlockNumbers>(hash)?,
        };

        if let Some(num) = block_num {
            Ok(self.tx.get::<tables::BlockBodyIndices>(num)?)
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
            BlockHashOrNumber::Hash(hash) => self.tx.get::<tables::BlockNumbers>(hash)?,
        };

        let Some(block_num) = block_num else { return Ok(None) };

        if let Some(header) = self.header(block_num.into())? {
            let body_indices = self
                .block_body_indices(block_num.into())?
                .ok_or(ProviderError::MissingBlockTxs(block_num))?;

            let body = self.transaction_hashes_in_range(Range::from(body_indices))?;
            let block = BlockWithTxHashes { header, body };

            Ok(Some(block))
        } else {
            Ok(None)
        }
    }

    fn blocks_in_range(&self, range: RangeInclusive<u64>) -> ProviderResult<Vec<Block>> {
        let total = range.end().saturating_sub(*range.start()) + 1;
        let mut blocks = Vec::with_capacity(total as usize);

        for num in range {
            if let Some(header) = self.header(num.into())? {
                let body_indices = self
                    .block_body_indices(num.into())?
                    .ok_or(ProviderError::MissingBlockBodyIndices(num))?;

                let body = self.transaction_in_range(Range::from(body_indices))?;
                blocks.push(Block { header, body })
            }
        }

        Ok(blocks)
    }
}

impl<Tx: DbTx> BlockStatusProvider for DbProvider<Tx> {
    fn block_status(&self, id: BlockHashOrNumber) -> ProviderResult<Option<FinalityStatus>> {
        match id {
            BlockHashOrNumber::Num(num) => {
                let status = self.tx.get::<tables::BlockStatusses>(num)?;
                Ok(status)
            }

            BlockHashOrNumber::Hash(hash) => {
                if let Some(num) = self.block_number_by_hash(hash)? {
                    let res = self.tx.get::<tables::BlockStatusses>(num)?;
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
        if let Some(num) = self.tx.get::<tables::TxNumbers>(hash)? {
            let sf_ref =
                self.tx.get::<tables::Transactions>(num)?.ok_or(ProviderError::MissingTx(num))?;

            let envelope: TxEnvelope =
                resolve_static_ref(&self.static_files, &sf_ref, |sf, o, l| {
                    sf.read_transaction(o, l)
                })?;

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
            let sf_ref = self.tx.get::<tables::Transactions>(i)?;

            if let Some(sf_ref) = sf_ref {
                let envelope: TxEnvelope =
                    resolve_static_ref(&self.static_files, &sf_ref, |sf, o, l| {
                        sf.read_transaction(o, l)
                    })?;

                let hash = self.get_tx_hash(i)?.ok_or(ProviderError::MissingTxHash(i))?;

                transactions.push(TxWithHash { hash, transaction: envelope.inner.into() });
            };
        }

        Ok(transactions)
    }

    fn transaction_block_num_and_hash(
        &self,
        hash: TxHash,
    ) -> ProviderResult<Option<(BlockNumber, BlockHash)>> {
        if let Some(num) = self.tx.get::<tables::TxNumbers>(hash)? {
            let block_num = self.get_tx_block(num)?.ok_or(ProviderError::MissingTxBlock(num))?;

            let block_hash =
                self.block_hash_by_num(block_num)?.ok_or(ProviderError::MissingBlockHash(num))?;

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

                let hash = self.get_tx_hash(num)?.ok_or(ProviderError::MissingTxHash(num))?;

                let sf_ref = self
                    .tx
                    .get::<tables::Transactions>(num)?
                    .ok_or(ProviderError::MissingTx(num))?;

                let envelope: TxEnvelope =
                    resolve_static_ref(&self.static_files, &sf_ref, |sf, o, l| {
                        sf.read_transaction(o, l)
                    })?;

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
            if let Some(hash) = self.get_tx_hash(i)? {
                hashes.push(hash);
            }
        }

        Ok(hashes)
    }

    fn total_transactions(&self) -> ProviderResult<usize> {
        Ok(self.tx.entries::<tables::Transactions>()?)
    }
}

impl<Tx: DbTx> TransactionStatusProvider for DbProvider<Tx> {
    fn transaction_status(&self, hash: TxHash) -> ProviderResult<Option<FinalityStatus>> {
        if let Some(tx_num) = self.tx.get::<tables::TxNumbers>(hash)? {
            let block_num =
                self.get_tx_block(tx_num)?.ok_or(ProviderError::MissingTxBlock(tx_num))?;

            let res = self.tx.get::<tables::BlockStatusses>(block_num)?;
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
        if let Some(num) = self.tx.get::<tables::TxNumbers>(hash)? {
            let sf_ref = self.tx.get::<tables::TxTraces>(num);
            match sf_ref {
                Ok(Some(r)) => {
                    match resolve_static_ref(&self.static_files, &r, |sf, o, l| {
                        sf.read_tx_trace(o, l)
                    }) {
                        Ok(execution) => return Ok(Some(execution)),
                        Err(ProviderError::StaticFile(StaticFileError::Codec(
                            CodecError::Decompress(err),
                        ))) => {
                            warn!(tx_num = %num, %err, "Failed to deserialize transaction trace from static files");
                            return Ok(None);
                        }
                        Err(ProviderError::Other(err)) => {
                            warn!(tx_num = %num, %err, "Failed to deserialize inline transaction trace");
                            return Ok(None);
                        }
                        Err(e) => return Err(e),
                    }
                }
                Ok(None) => Ok(None),
                Err(katana_db::error::DatabaseError::Codec(CodecError::Decompress(err))) => {
                    warn!(tx_num = %num, %err, "Failed to deserialize transaction trace ref");
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
            let sf_ref = self.tx.get::<tables::TxTraces>(i);
            let trace = match sf_ref {
                Ok(Some(r)) => {
                    match resolve_static_ref(&self.static_files, &r, |sf, o, l| {
                        sf.read_tx_trace(o, l)
                    }) {
                        Ok(trace) => Some(trace),
                        Err(ProviderError::StaticFile(StaticFileError::Codec(
                            CodecError::Decompress(err),
                        ))) => {
                            warn!(tx_num = %i, %err, "Failed to deserialize transaction trace from static files");
                            None
                        }
                        Err(ProviderError::Other(err)) => {
                            warn!(tx_num = %i, %err, "Failed to deserialize inline transaction trace");
                            None
                        }
                        Err(e) => return Err(e),
                    }
                }
                Ok(None) => None,
                Err(katana_db::error::DatabaseError::Codec(CodecError::Decompress(err))) => {
                    warn!(tx_num = %i, %err, "Failed to deserialize transaction trace ref");
                    None
                }
                Err(e) => return Err(e.into()),
            };

            if let Some(trace) = trace {
                traces.push(trace);
            }
        }

        Ok(traces)
    }
}

impl<Tx: DbTx> ReceiptProvider for DbProvider<Tx> {
    fn receipt_by_hash(&self, hash: TxHash) -> ProviderResult<Option<Receipt>> {
        if let Some(num) = self.tx.get::<tables::TxNumbers>(hash)? {
            let sf_ref = self
                .tx
                .get::<tables::Receipts>(num)?
                .ok_or(ProviderError::MissingTxReceipt(num))?;

            let envelope: ReceiptEnvelope =
                resolve_static_ref(&self.static_files, &sf_ref, |sf, o, l| sf.read_receipt(o, l))?;

            Ok(Some(Receipt::from(envelope)))
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
                let sf_ref = self.tx.get::<tables::Receipts>(i)?;
                if let Some(sf_ref) = sf_ref {
                    let envelope: ReceiptEnvelope =
                        resolve_static_ref(&self.static_files, &sf_ref, |sf, o, l| {
                            sf.read_receipt(o, l)
                        })?;
                    receipts.push(envelope.into());
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
    /// Store a single block's data (header, transactions, receipts, traces, state updates,
    /// classes).
    ///
    /// Operates in two modes based on whether block numbers are sequential:
    ///
    /// - **Sequential mode** (production): Appends heavy data to static files and stores
    ///   `StaticFileRef::StaticFile` pointers in MDBX. Fixed-size indexes (block hashes, tx hashes,
    ///   tx-to-block) are written to static files only; MDBX fallback tables (`BlockHashes`,
    ///   `TxHashes`, `TxBlocks`) are skipped.
    ///
    /// - **Fork mode** (non-sequential): Compresses data and stores `StaticFileRef::Inline` in
    ///   MDBX. All index tables are written to MDBX.
    ///
    /// Static files are NOT fsynced here — that happens in [`MutableProvider::commit`].
    /// On crash before commit, orphaned static file data is truncated on next startup.
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
        let tx_offset = self.tx.entries::<tables::Transactions>()? as u64;
        let block_body_indices = StoredBlockBodyIndices { tx_offset, tx_count };

        // Check if we can append sequentially to static files.
        let is_sequential = self
            .static_files
            .blocks
            .block_hashes
            .count()
            .map_err(|e| ProviderError::StaticFile(StaticFileError::Io(e)))?
            == block_number;

        // -- MDBX: write reverse indexes and mutable data --
        self.tx.put::<tables::BlockNumbers>(block_hash, block_number)?;
        self.tx.put::<tables::BlockStatusses>(block_number, block.status)?;

        // BlockHashes: written to MDBX only in non-sequential mode (fork).
        // In sequential mode, it's in static files only.
        if !is_sequential {
            self.tx.put::<tables::BlockHashes>(block_number, block_hash)?;
        }

        if is_sequential {
            // Append variable-size data to static files, store pointers in MDBX.
            let (h_off, h_len) = self
                .static_files
                .append_header(VersionedHeader::from(block_header))
                .map_err(ProviderError::StaticFile)?;
            self.tx.put::<tables::Headers>(block_number, StaticFileRef::pointer(h_off, h_len))?;

            // BlockBodyIndices is small (~10B), stored directly in MDBX (no pointer).
            self.tx.put::<tables::BlockBodyIndices>(block_number, block_body_indices.clone())?;

            let (su_off, su_len) = self
                .static_files
                .append_block_state_update(StateUpdateEnvelope::from(state_updates.clone()))
                .map_err(ProviderError::StaticFile)?;
            self.tx.put::<tables::BlockStateUpdates>(
                block_number,
                StaticFileRef::pointer(su_off, su_len),
            )?;

            // Append fixed-size block hash.
            self.static_files
                .append_block_hash(block_number, block_hash)
                .map_err(ProviderError::StaticFile)?;
        } else {
            // Non-sequential (fork mode): compress and store inline in MDBX.
            let header_bytes = compress_value(VersionedHeader::from(block_header))?;
            self.tx.put::<tables::Headers>(block_number, StaticFileRef::inline(header_bytes))?;

            self.tx.put::<tables::BlockBodyIndices>(block_number, block_body_indices.clone())?;

            let su_bytes = compress_value(StateUpdateEnvelope::from(state_updates.clone()))?;
            self.tx
                .put::<tables::BlockStateUpdates>(block_number, StaticFileRef::inline(su_bytes))?;
        }

        // Store base transaction details
        for (i, transaction) in transactions.into_iter().enumerate() {
            let tx_number = tx_offset + i as u64;
            let tx_hash = transaction.hash;

            self.tx.put::<tables::TxNumbers>(tx_hash, tx_number)?;

            // TxHashes/TxBlocks: written to MDBX only in non-sequential mode (fork).
            if !is_sequential {
                self.tx.put::<tables::TxHashes>(tx_number, tx_hash)?;
                self.tx.put::<tables::TxBlocks>(tx_number, block_number)?;
            }

            let tx_envelope = TxEnvelope::from(VersionedTx::from(transaction.transaction));

            if is_sequential {
                let receipt_envelope = ReceiptEnvelope::from(
                    receipts.get(i).cloned().expect("missing receipt for sequential tx"),
                );
                let execution =
                    executions.get(i).cloned().expect("missing execution for sequential tx");

                // Append to static files, store pointers in MDBX.
                let (tx_off, tx_len) = self
                    .static_files
                    .append_transaction(tx_envelope)
                    .map_err(ProviderError::StaticFile)?;
                self.tx.put::<tables::Transactions>(
                    tx_number,
                    StaticFileRef::pointer(tx_off, tx_len),
                )?;

                let (r_off, r_len) = self
                    .static_files
                    .append_receipt(receipt_envelope)
                    .map_err(ProviderError::StaticFile)?;
                self.tx.put::<tables::Receipts>(tx_number, StaticFileRef::pointer(r_off, r_len))?;

                let (t_off, t_len) = self
                    .static_files
                    .append_tx_trace(execution)
                    .map_err(ProviderError::StaticFile)?;
                self.tx.put::<tables::TxTraces>(tx_number, StaticFileRef::pointer(t_off, t_len))?;

                // Fixed-size: tx hash and tx-to-block mapping.
                self.static_files
                    .append_tx_hash(tx_number, tx_hash)
                    .map_err(ProviderError::StaticFile)?;
                self.static_files
                    .append_tx_block(tx_number, block_number)
                    .map_err(ProviderError::StaticFile)?;
            } else {
                // Non-sequential (fork mode): compress and store inline.
                let tx_bytes = compress_value(tx_envelope)?;
                self.tx.put::<tables::Transactions>(tx_number, StaticFileRef::inline(tx_bytes))?;

                if let Some(receipt) = receipts.get(i) {
                    let r_bytes = compress_value(ReceiptEnvelope::from(receipt.clone()))?;
                    self.tx.put::<tables::Receipts>(tx_number, StaticFileRef::inline(r_bytes))?;
                }

                if let Some(execution) = executions.get(i) {
                    let t_bytes = compress_value(execution.clone())?;
                    self.tx.put::<tables::TxTraces>(tx_number, StaticFileRef::inline(t_bytes))?;
                }
            }
        }

        // Note: static files are synced in commit(), not here, to avoid
        // per-block fsync overhead when inserting many blocks in a batch.

        // insert all class artifacts
        for (class_hash, class) in classes {
            self.tx.put::<tables::Classes>(class_hash, class.into())?;
        }

        // insert compiled class hashes and declarations for declared classes
        for (class_hash, compiled_hash) in state_updates.declared_classes {
            self.tx.put::<tables::CompiledClassHashes>(class_hash, compiled_hash)?;
            self.tx.put::<tables::ClassDeclarationBlock>(class_hash, block_number)?;
            self.tx.put::<tables::ClassDeclarations>(block_number, class_hash)?;
        }

        // insert declarations for deprecated declared classes
        for class_hash in state_updates.deprecated_declared_classes {
            self.tx.put::<tables::ClassDeclarationBlock>(class_hash, block_number)?;
            self.tx.put::<tables::ClassDeclarations>(block_number, class_hash)?;
        }

        // insert migrated class hashes
        for (class_hash, compiled_class_hash) in state_updates.migrated_compiled_classes {
            let entry = MigratedCompiledClassHash { class_hash, compiled_class_hash };
            self.tx.put::<tables::MigratedCompiledClassHashes>(block_number, entry)?;
        }

        Ok(())
    }

    /// Two-phase batch insertion optimized for the sync pipeline.
    ///
    /// Phase 1: Appends ALL block/tx data to static files (sequential I/O), collecting
    /// the resulting pointers in memory.
    /// Phase 2: Writes ALL MDBX entries (pointers + indexes) in one pass.
    ///
    /// This improves I/O locality compared to per-block interleaved writes, and
    /// pre-sizes static file buffers to avoid reallocations during the batch.
    #[allow(clippy::type_complexity)]
    pub fn insert_block_data_batch(
        &self,
        blocks: Vec<(
            SealedBlockWithStatus,
            StateUpdatesWithClasses,
            Vec<Receipt>,
            Vec<TypedTransactionExecutionInfo>,
        )>,
    ) -> ProviderResult<()> {
        if blocks.is_empty() {
            return Ok(());
        }

        let total_blocks = blocks.len();
        let total_txs: usize = blocks.iter().map(|(b, _, _, _)| b.block.body.len()).sum();

        let first_block_num = blocks[0].0.block.header.number;
        let is_sequential = self
            .static_files
            .blocks
            .block_hashes
            .count()
            .map_err(|e| ProviderError::StaticFile(StaticFileError::Io(e)))?
            == first_block_num;

        if is_sequential {
            self.static_files
                .reserve_for_batch(total_blocks, total_txs)
                .map_err(ProviderError::StaticFile)?;
        }

        // ---- Phase 1: Static file appends (sequential I/O) ----
        // Collect all pointer data in flat vectors for phase 2.

        let mut tx_counter = self.tx.entries::<tables::Transactions>()? as u64;

        // Block-level collected data.
        let mut block_metas: Vec<(
            BlockNumber,
            BlockHash,
            FinalityStatus,
            StoredBlockBodyIndices,
            StaticFileRef,
            StaticFileRef,
        )> = Vec::with_capacity(total_blocks);
        // Tx-level collected data.
        let mut tx_metas: Vec<(
            TxNumber,
            TxHash,
            BlockNumber,
            StaticFileRef,
            Option<StaticFileRef>,
            Option<StaticFileRef>,
        )> = Vec::with_capacity(total_txs);
        // Class data (passed through to phase 2).
        let mut class_data: Vec<(BlockNumber, StateUpdatesWithClasses)> =
            Vec::with_capacity(total_blocks);

        for (block, states, receipts, executions) in blocks {
            let block_hash = block.block.hash;
            let block_number = block.block.header.number;
            let block_header = block.block.header;
            let transactions = block.block.body;
            let status = block.status;

            let tx_count = transactions.len() as u64;
            let tx_offset = tx_counter;
            let body_indices = StoredBlockBodyIndices { tx_offset, tx_count };

            let (header_ref, su_ref) = if is_sequential {
                let (h_off, h_len) = self
                    .static_files
                    .append_header(VersionedHeader::from(block_header))
                    .map_err(ProviderError::StaticFile)?;
                let (su_off, su_len) = self
                    .static_files
                    .append_block_state_update(StateUpdateEnvelope::from(
                        states.state_updates.clone(),
                    ))
                    .map_err(ProviderError::StaticFile)?;
                self.static_files
                    .append_block_hash(block_number, block_hash)
                    .map_err(ProviderError::StaticFile)?;

                (StaticFileRef::pointer(h_off, h_len), StaticFileRef::pointer(su_off, su_len))
            } else {
                (
                    StaticFileRef::inline(compress_value(VersionedHeader::from(block_header))?),
                    StaticFileRef::inline(compress_value(StateUpdateEnvelope::from(
                        states.state_updates.clone(),
                    ))?),
                )
            };

            block_metas.push((block_number, block_hash, status, body_indices, header_ref, su_ref));

            for (i, transaction) in transactions.into_iter().enumerate() {
                let tx_number = tx_offset + i as u64;
                let tx_hash = transaction.hash;
                let tx_envelope = TxEnvelope::from(VersionedTx::from(transaction.transaction));

                let (tx_ref, r_ref, t_ref) = if is_sequential {
                    let (tx_off, tx_len) = self
                        .static_files
                        .append_transaction(tx_envelope)
                        .map_err(ProviderError::StaticFile)?;
                    let (r_off, r_len) = self
                        .static_files
                        .append_receipt(ReceiptEnvelope::from(
                            receipts.get(i).cloned().expect("missing receipt"),
                        ))
                        .map_err(ProviderError::StaticFile)?;
                    let (t_off, t_len) = self
                        .static_files
                        .append_tx_trace(executions.get(i).cloned().expect("missing execution"))
                        .map_err(ProviderError::StaticFile)?;
                    self.static_files
                        .append_tx_hash(tx_number, tx_hash)
                        .map_err(ProviderError::StaticFile)?;
                    self.static_files
                        .append_tx_block(tx_number, block_number)
                        .map_err(ProviderError::StaticFile)?;

                    (
                        StaticFileRef::pointer(tx_off, tx_len),
                        Some(StaticFileRef::pointer(r_off, r_len)),
                        Some(StaticFileRef::pointer(t_off, t_len)),
                    )
                } else {
                    let tx_ref = StaticFileRef::inline(compress_value(tx_envelope)?);
                    let r_ref = receipts
                        .get(i)
                        .map(|r| {
                            compress_value(ReceiptEnvelope::from(r.clone()))
                                .map(StaticFileRef::inline)
                        })
                        .transpose()?;
                    let t_ref = executions
                        .get(i)
                        .map(|e| compress_value(e.clone()).map(StaticFileRef::inline))
                        .transpose()?;
                    (tx_ref, r_ref, t_ref)
                };

                tx_metas.push((tx_number, tx_hash, block_number, tx_ref, r_ref, t_ref));
            }

            tx_counter += tx_count;
            class_data.push((block_number, states));
        }

        // ---- Phase 2: MDBX writes (B-tree inserts) ----

        for (block_number, block_hash, status, body_indices, header_ref, su_ref) in block_metas {
            self.tx.put::<tables::BlockNumbers>(block_hash, block_number)?;
            self.tx.put::<tables::BlockStatusses>(block_number, status)?;
            self.tx.put::<tables::Headers>(block_number, header_ref)?;
            self.tx.put::<tables::BlockBodyIndices>(block_number, body_indices)?;
            self.tx.put::<tables::BlockStateUpdates>(block_number, su_ref)?;

            if !is_sequential {
                self.tx.put::<tables::BlockHashes>(block_number, block_hash)?;
            }
        }

        for (tx_number, tx_hash, block_number, tx_ref, r_ref, t_ref) in tx_metas {
            self.tx.put::<tables::TxNumbers>(tx_hash, tx_number)?;
            self.tx.put::<tables::Transactions>(tx_number, tx_ref)?;
            if let Some(r) = r_ref {
                self.tx.put::<tables::Receipts>(tx_number, r)?;
            }
            if let Some(t) = t_ref {
                self.tx.put::<tables::TxTraces>(tx_number, t)?;
            }
            if !is_sequential {
                self.tx.put::<tables::TxHashes>(tx_number, tx_hash)?;
                self.tx.put::<tables::TxBlocks>(tx_number, block_number)?;
            }
        }

        for (block_number, states) in class_data {
            let StateUpdatesWithClasses { state_updates, classes } = states;
            for (class_hash, class) in classes {
                self.tx.put::<tables::Classes>(class_hash, class.into())?;
            }
            for (class_hash, compiled_hash) in state_updates.declared_classes {
                self.tx.put::<tables::CompiledClassHashes>(class_hash, compiled_hash)?;
                self.tx.put::<tables::ClassDeclarationBlock>(class_hash, block_number)?;
                self.tx.put::<tables::ClassDeclarations>(block_number, class_hash)?;
            }
            for class_hash in state_updates.deprecated_declared_classes {
                self.tx.put::<tables::ClassDeclarationBlock>(class_hash, block_number)?;
                self.tx.put::<tables::ClassDeclarations>(block_number, class_hash)?;
            }
            for (class_hash, compiled_class_hash) in state_updates.migrated_compiled_classes {
                let entry = MigratedCompiledClassHash { class_hash, compiled_class_hash };
                self.tx.put::<tables::MigratedCompiledClassHashes>(block_number, entry)?;
            }
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
                    self.tx.put::<tables::StorageChangeHistory>(
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
                self.tx.put::<tables::ClassChangeHistory>(block_number, class_change_key)?;
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
                self.tx.put::<tables::ClassChangeHistory>(block_number, class_change_key)?;
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
                self.tx.put::<tables::NonceChangeHistory>(block_number, nonce_change_key)?;
            }
        }

        // Flush accumulated storage change sets (one write per key).
        for (key, block_list) in storage_change_sets {
            self.tx.put::<tables::StorageChangeSet>(key, block_list)?;
        }

        // Flush latest storage values (one write per (addr, key)).
        {
            let mut storage_cursor = self.tx.cursor_dup_mut::<tables::ContractStorage>()?;
            for ((addr, _), entry) in &latest_storage {
                storage_cursor.upsert(*addr, *entry)?;
            }
        }

        // Flush accumulated contract info change sets (one write per address).
        for (addr, change_set) in contract_info_change_sets {
            self.tx.put::<tables::ContractInfoChangeSet>(addr, change_set)?;
        }

        // Flush latest contract info (one write per address).
        for (addr, info) in latest_contract_info {
            self.tx.put::<tables::ContractInfo>(addr, info)?;
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
            let mut storage_cursor = self.tx.cursor_dup_mut::<tables::ContractStorage>()?;
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
                    let list = self.tx.get::<tables::StorageChangeSet>(changeset_key.clone())?;

                    let updated_list = match list {
                        Some(mut list) => {
                            list.insert(block_number);
                            list
                        }
                        // create a new block list if it doesn't yet exist, and insert the block
                        // number
                        None => BlockChangeList::from([block_number]),
                    };

                    self.tx.put::<tables::StorageChangeSet>(changeset_key, updated_list)?;
                    storage_cursor.upsert(*addr, entry)?;

                    let storage_change_sharded_key =
                        ContractStorageKey { contract_address: *addr, key: entry.key };

                    self.tx.put::<tables::StorageChangeHistory>(
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
            let value = if let Some(info) = self.tx.get::<tables::ContractInfo>(*addr)? {
                GenericContractInfo { class_hash: *class_hash, ..info }
            } else {
                GenericContractInfo { class_hash: *class_hash, ..Default::default() }
            };

            let new_change_set = if let Some(mut change_set) =
                self.tx.get::<tables::ContractInfoChangeSet>(*addr)?
            {
                change_set.class_change_list.insert(block_number);
                change_set
            } else {
                ContractInfoChangeList {
                    class_change_list: BlockChangeList::from([block_number]),
                    ..Default::default()
                }
            };

            self.tx.put::<tables::ContractInfo>(*addr, value)?;

            let class_change_key = ContractClassChange::deployed(*addr, *class_hash);
            self.tx.put::<tables::ClassChangeHistory>(block_number, class_change_key)?;
            self.tx.put::<tables::ContractInfoChangeSet>(*addr, new_change_set)?;
        }

        for (addr, new_class_hash) in &state_updates.replaced_classes {
            let info = if let Some(info) = self.tx.get::<tables::ContractInfo>(*addr)? {
                GenericContractInfo { class_hash: *new_class_hash, ..info }
            } else {
                GenericContractInfo { class_hash: *new_class_hash, ..Default::default() }
            };

            let new_change_set = if let Some(mut change_set) =
                self.tx.get::<tables::ContractInfoChangeSet>(*addr)?
            {
                change_set.class_change_list.insert(block_number);
                change_set
            } else {
                ContractInfoChangeList {
                    class_change_list: BlockChangeList::from([block_number]),
                    ..Default::default()
                }
            };

            self.tx.put::<tables::ContractInfo>(*addr, info)?;

            let class_change_key = ContractClassChange::replaced(*addr, *new_class_hash);
            self.tx.put::<tables::ClassChangeHistory>(block_number, class_change_key)?;
            self.tx.put::<tables::ContractInfoChangeSet>(*addr, new_change_set)?;
        }

        for (addr, nonce) in &state_updates.nonce_updates {
            let value = if let Some(info) = self.tx.get::<tables::ContractInfo>(*addr)? {
                GenericContractInfo { nonce: *nonce, ..info }
            } else {
                GenericContractInfo { nonce: *nonce, ..Default::default() }
            };

            let new_change_set = if let Some(mut change_set) =
                self.tx.get::<tables::ContractInfoChangeSet>(*addr)?
            {
                change_set.nonce_change_list.insert(block_number);
                change_set
            } else {
                ContractInfoChangeList {
                    nonce_change_list: BlockChangeList::from([block_number]),
                    ..Default::default()
                }
            };

            self.tx.put::<tables::ContractInfo>(*addr, value)?;

            let nonce_change_key = ContractNonceChange { contract_address: *addr, nonce: *nonce };
            self.tx.put::<tables::NonceChangeHistory>(block_number, nonce_change_key)?;
            self.tx.put::<tables::ContractInfoChangeSet>(*addr, new_change_set)?;
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
        let result = self.tx.get::<tables::StageExecutionCheckpoints>(id.to_string())?;
        Ok(result.map(|x| x.block))
    }

    fn set_execution_checkpoint(&self, id: &str, block_number: BlockNumber) -> ProviderResult<()> {
        let key = id.to_string();
        let value = ExecutionCheckpoint { block: block_number };
        self.tx.put::<tables::StageExecutionCheckpoints>(key, value)?;
        Ok(())
    }

    fn prune_checkpoint(&self, id: &str) -> ProviderResult<Option<BlockNumber>> {
        let result = self.tx.get::<tables::StagePruningCheckpoints>(id.to_string())?;
        Ok(result.map(|x| x.block))
    }

    fn set_prune_checkpoint(&self, id: &str, block_number: BlockNumber) -> ProviderResult<()> {
        let key = id.to_string();
        let value = PruningCheckpoint { block: block_number };
        self.tx.put::<tables::StagePruningCheckpoints>(key, value)?;
        Ok(())
    }
}

pub const STATE_HISTORY_RETENTION_KEY: u64 = 0;
pub const STATE_TRIE_HISTORY_RETENTION_KEY: u64 = 1;

impl<Tx: DbTxMut> HistoricalStateRetentionProvider for DbProvider<Tx> {
    fn earliest_available_state_block(&self) -> ProviderResult<Option<BlockNumber>> {
        let key = STATE_HISTORY_RETENTION_KEY;
        let result = self.tx.get::<tables::StateHistoryRetention>(key)?;
        Ok(result.map(|retention| retention.earliest_available_block))
    }

    fn set_earliest_available_state_block(&self, block_number: BlockNumber) -> ProviderResult<()> {
        let key = STATE_HISTORY_RETENTION_KEY;
        let value = HistoricalStateRetention { earliest_available_block: block_number };
        self.tx.put::<tables::StateHistoryRetention>(key, value)?;
        Ok(())
    }

    fn earliest_available_state_trie_block(&self) -> ProviderResult<Option<BlockNumber>> {
        let key = STATE_TRIE_HISTORY_RETENTION_KEY;
        let result = self.tx.get::<tables::StateHistoryRetention>(key)?;
        Ok(result.map(|retention| retention.earliest_available_block))
    }

    fn set_earliest_available_state_trie_block(
        &self,
        block_number: BlockNumber,
    ) -> ProviderResult<()> {
        let key = STATE_TRIE_HISTORY_RETENTION_KEY;
        let value = HistoricalStateRetention { earliest_available_block: block_number };
        self.tx.put::<tables::StateHistoryRetention>(key, value)?;
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

    fn create_dummy_block_1() -> SealedBlockWithStatus {
        let header = Header { parent_hash: 200u8.into(), number: 1, ..Default::default() };
        let block = Block {
            header,
            body: vec![TxWithHash {
                hash: 25u8.into(),
                transaction: Tx::Invoke(InvokeTx::V1(Default::default())),
            }],
        }
        .seal();
        SealedBlockWithStatus { block, status: FinalityStatus::AcceptedOnL2 }
    }

    #[test]
    fn storage_updated_correctly() {
        let provider = create_db_provider();
        let provider = provider.provider_mut();

        let block0 = create_dummy_block();
        let block1 = create_dummy_block_1();
        let state_updates1 = create_dummy_state_updates();
        let state_updates2 = create_dummy_state_updates_2();

        // insert block 0
        provider
            .insert_block_with_states_and_receipts(
                block0,
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

        // insert block 1
        provider
            .insert_block_with_states_and_receipts(
                block1,
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
