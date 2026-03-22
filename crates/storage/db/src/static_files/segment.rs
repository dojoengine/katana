use std::io;
use std::path::Path;

use super::column::{DataColumn, FixedColumn};
use super::store::{AnyStore, FileStore, FileStoreConfig, MemoryStore, StaticStore};
use crate::codecs::{Compress, Decompress};
use crate::error::CodecError;

/// Block-indexed segment grouping block-level static columns.
pub struct BlockSegment<S: StaticStore> {
    /// Fixed 32B per block. In sequential mode, this is the primary store; reads fall back to
    /// MDBX `BlockHashes` table in fork mode where this column is not written.
    pub block_hashes: FixedColumn<S>,
    /// Variable-size — (offset, length) stored in MDBX as StaticFileRef.
    pub headers: DataColumn<S>,
    /// Variable-size data column (kept for compatibility but currently unused —
    /// BlockBodyIndices is stored directly in MDBX as it's too small for pointer indirection).
    pub block_body_indices: DataColumn<S>,
    /// Variable-size — (offset, length) stored in MDBX as StaticFileRef.
    pub block_state_updates: DataColumn<S>,
}

/// Transaction-indexed segment grouping transaction-level static columns.
pub struct TxSegment<S: StaticStore> {
    /// Fixed 32B per tx. Primary store in sequential mode; falls back to MDBX `TxHashes` in fork
    /// mode.
    pub tx_hashes: FixedColumn<S>,
    /// Fixed 8B per tx. Primary store in sequential mode; falls back to MDBX `TxBlocks` in fork
    /// mode.
    pub tx_blocks: FixedColumn<S>,
    /// Variable-size — (offset, length) stored in MDBX as StaticFileRef.
    pub transactions: DataColumn<S>,
    /// Variable-size — (offset, length) stored in MDBX as StaticFileRef.
    pub receipts: DataColumn<S>,
    /// Variable-size — (offset, length) stored in MDBX as StaticFileRef.
    pub tx_traces: DataColumn<S>,
}

/// Top-level container for all static file data.
///
/// MDBX is the authority for what data exists. Static files are the storage backend
/// for heavy immutable data. Each variable-size column has a corresponding MDBX table
/// storing `StaticFileRef` pointers. Fixed-size columns are gated by the existence of
/// a related pointer entry.
///
/// In **sequential mode** (production), data is appended to `.dat` files and MDBX stores
/// `StaticFileRef::StaticFile` pointers. In **fork mode** (non-sequential block numbers),
/// static files are not written; MDBX stores `StaticFileRef::Inline` with compressed data.
pub struct StaticFiles<S: StaticStore> {
    pub blocks: BlockSegment<S>,
    pub transactions: TxSegment<S>,
}

impl<S: StaticStore> std::fmt::Debug for StaticFiles<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StaticFiles").finish_non_exhaustive()
    }
}

// -- Builder --

/// Builder for configuring and creating a [`StaticFiles`] instance.
#[derive(Debug, Clone)]
pub struct StaticFilesBuilder {
    mode: StaticFilesMode,
    file_store_config: FileStoreConfig,
}

#[derive(Debug, Clone)]
enum StaticFilesMode {
    File(std::path::PathBuf),
    Memory,
}

impl StaticFilesBuilder {
    /// Create a new builder. Defaults to in-memory mode.
    pub fn new() -> Self {
        Self { mode: StaticFilesMode::Memory, file_store_config: FileStoreConfig::default() }
    }

    /// Use file-backed storage at the given directory.
    pub fn file(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.mode = StaticFilesMode::File(path.into());
        self
    }

    /// Use in-memory storage (tests, ephemeral mode).
    pub fn memory(mut self) -> Self {
        self.mode = StaticFilesMode::Memory;
        self
    }

    /// Set the initial write buffer capacity per column (default: 64KB).
    pub fn write_buffer_size(mut self, size: usize) -> Self {
        self.file_store_config.write_buffer_size = size;
        self
    }

    /// Set the auto-flush threshold per column (default: 256KB).
    pub fn flush_threshold(mut self, threshold: usize) -> Self {
        self.file_store_config.flush_threshold = threshold;
        self
    }

    /// Disable mmap for reads (fall back to pread only).
    pub fn no_mmap(mut self) -> Self {
        self.file_store_config.use_mmap = false;
        self
    }

    /// Build the static files instance.
    pub fn build(self) -> io::Result<StaticFiles<AnyStore>> {
        match self.mode {
            StaticFilesMode::File(base_path) => {
                let blocks_path = base_path.join("blocks");
                let txs_path = base_path.join("transactions");
                std::fs::create_dir_all(&blocks_path)?;
                std::fs::create_dir_all(&txs_path)?;

                let cfg = &self.file_store_config;
                let open = |dir: &Path, name: &str| -> io::Result<AnyStore> {
                    Ok(AnyStore::File(FileStore::open_with_config(&dir.join(name), cfg.clone())?))
                };

                let blocks = BlockSegment {
                    block_hashes: FixedColumn::new(open(&blocks_path, "block_hashes.dat")?, 32),
                    headers: DataColumn::new(open(&blocks_path, "headers.dat")?),
                    block_body_indices: DataColumn::new(open(
                        &blocks_path,
                        "block_body_indices.dat",
                    )?),
                    block_state_updates: DataColumn::new(open(
                        &blocks_path,
                        "block_state_updates.dat",
                    )?),
                };

                let transactions = TxSegment {
                    tx_hashes: FixedColumn::new(open(&txs_path, "tx_hashes.dat")?, 32),
                    tx_blocks: FixedColumn::new(open(&txs_path, "tx_blocks.dat")?, 8),
                    transactions: DataColumn::new(open(&txs_path, "transactions.dat")?),
                    receipts: DataColumn::new(open(&txs_path, "receipts.dat")?),
                    tx_traces: DataColumn::new(open(&txs_path, "tx_traces.dat")?),
                };

                Ok(StaticFiles { blocks, transactions })
            }
            StaticFilesMode::Memory => Ok(StaticFiles::new_in_memory()),
        }
    }
}

impl Default for StaticFilesBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// -- Constructors (convenience methods delegating to builder) --

impl StaticFiles<AnyStore> {
    /// Open file-backed static files at the given directory (production).
    pub fn open_file(base_path: &Path) -> io::Result<Self> {
        StaticFilesBuilder::new().file(base_path).build()
    }

    /// Create in-memory static files (tests, ephemeral mode).
    pub fn in_memory() -> Self {
        StaticFilesBuilder::new().memory().build().expect("in-memory cannot fail")
    }

    /// Internal constructor for in-memory static files (avoids infinite recursion
    /// with the builder).
    fn new_in_memory() -> Self {
        let mem = || AnyStore::Memory(MemoryStore::new());

        let blocks = BlockSegment {
            block_hashes: FixedColumn::new(mem(), 32),
            headers: DataColumn::new(mem()),
            block_body_indices: DataColumn::new(mem()),
            block_state_updates: DataColumn::new(mem()),
        };

        let transactions = TxSegment {
            tx_hashes: FixedColumn::new(mem(), 32),
            tx_blocks: FixedColumn::new(mem(), 8),
            transactions: DataColumn::new(mem()),
            receipts: DataColumn::new(mem()),
            tx_traces: DataColumn::new(mem()),
        };

        Self { blocks, transactions }
    }
}

// -- Helpers --

/// Compress a value and return the raw bytes.
fn compress_value<T: Compress>(value: T) -> Result<Vec<u8>, CodecError> {
    Ok(value.compress()?.into())
}

/// Decompress raw bytes into a typed value.
fn decompress_value<T: Decompress>(bytes: &[u8]) -> Result<T, CodecError> {
    T::decompress(bytes)
}

// -- Read/Write API --
//
// Variable-size writes return (offset, length) for the caller to store in MDBX.
// Variable-size reads take (offset, length) from MDBX.
// Fixed-size reads/writes use the sequential key directly.

impl<S: StaticStore> StaticFiles<S> {
    // Variable-size writes return `(offset, length)` for the caller to store as
    // `StaticFileRef::pointer(offset, length)` in MDBX.

    /// Append a compressed header. Returns `(offset, length)`.
    pub fn append_header<T: Compress>(&self, header: T) -> Result<(u64, u32), StaticFileError> {
        let bytes = compress_value(header)?;
        Ok(self.blocks.headers.append(&bytes)?)
    }

    /// Append compressed block body indices. Returns `(offset, length)`.
    pub fn append_block_body_indices<T: Compress>(
        &self,
        indices: T,
    ) -> Result<(u64, u32), StaticFileError> {
        let bytes = compress_value(indices)?;
        Ok(self.blocks.block_body_indices.append(&bytes)?)
    }

    /// Append a compressed block state update. Returns `(offset, length)`.
    pub fn append_block_state_update<T: Compress>(
        &self,
        update: T,
    ) -> Result<(u64, u32), StaticFileError> {
        let bytes = compress_value(update)?;
        Ok(self.blocks.block_state_updates.append(&bytes)?)
    }

    // ---- Block-level fixed-size writes ----

    /// Append a block hash at the given block number (fixed 32B).
    pub fn append_block_hash(
        &self,
        block_number: u64,
        hash: katana_primitives::block::BlockHash,
    ) -> Result<(), StaticFileError> {
        self.blocks.block_hashes.append(block_number, &hash.to_bytes_be())?;
        Ok(())
    }

    // ---- Transaction-level variable-size writes (return offset+length) ----

    /// Append a compressed transaction. Returns `(offset, length)`.
    pub fn append_transaction<T: Compress>(&self, tx: T) -> Result<(u64, u32), StaticFileError> {
        let bytes = compress_value(tx)?;
        Ok(self.transactions.transactions.append(&bytes)?)
    }

    /// Append a compressed receipt. Returns `(offset, length)`.
    pub fn append_receipt<T: Compress>(&self, receipt: T) -> Result<(u64, u32), StaticFileError> {
        let bytes = compress_value(receipt)?;
        Ok(self.transactions.receipts.append(&bytes)?)
    }

    /// Append a compressed transaction trace. Returns `(offset, length)`.
    pub fn append_tx_trace<T: Compress>(&self, trace: T) -> Result<(u64, u32), StaticFileError> {
        let bytes = compress_value(trace)?;
        Ok(self.transactions.tx_traces.append(&bytes)?)
    }

    // ---- Transaction-level fixed-size writes ----

    /// Append a transaction hash at the given tx number (fixed 32B).
    pub fn append_tx_hash(
        &self,
        tx_number: u64,
        hash: katana_primitives::transaction::TxHash,
    ) -> Result<(), StaticFileError> {
        self.transactions.tx_hashes.append(tx_number, &hash.to_bytes_be())?;
        Ok(())
    }

    /// Append a tx-to-block mapping at the given tx number (fixed 8B).
    pub fn append_tx_block(
        &self,
        tx_number: u64,
        block_number: u64,
    ) -> Result<(), StaticFileError> {
        self.transactions.tx_blocks.append(tx_number, &block_number.to_be_bytes())?;
        Ok(())
    }

    // Variable-size reads take `(offset, length)` from the MDBX `StaticFileRef` pointer.

    /// Read and decompress a header at the given static file position.
    pub fn read_header<T: Decompress>(
        &self,
        offset: u64,
        length: u32,
    ) -> Result<T, StaticFileError> {
        let bytes = self.blocks.headers.read(offset, length)?;
        Ok(decompress_value(&bytes)?)
    }

    /// Read and decompress block body indices at the given static file position.
    pub fn read_block_body_indices<T: Decompress>(
        &self,
        offset: u64,
        length: u32,
    ) -> Result<T, StaticFileError> {
        let bytes = self.blocks.block_body_indices.read(offset, length)?;
        Ok(decompress_value(&bytes)?)
    }

    /// Read and decompress a block state update at the given static file position.
    pub fn read_block_state_update<T: Decompress>(
        &self,
        offset: u64,
        length: u32,
    ) -> Result<T, StaticFileError> {
        let bytes = self.blocks.block_state_updates.read(offset, length)?;
        Ok(decompress_value(&bytes)?)
    }

    /// Read and decompress a transaction at the given static file position.
    pub fn read_transaction<T: Decompress>(
        &self,
        offset: u64,
        length: u32,
    ) -> Result<T, StaticFileError> {
        let bytes = self.transactions.transactions.read(offset, length)?;
        Ok(decompress_value(&bytes)?)
    }

    /// Read and decompress a receipt at the given static file position.
    pub fn read_receipt<T: Decompress>(
        &self,
        offset: u64,
        length: u32,
    ) -> Result<T, StaticFileError> {
        let bytes = self.transactions.receipts.read(offset, length)?;
        Ok(decompress_value(&bytes)?)
    }

    /// Read and decompress a transaction trace at the given static file position.
    pub fn read_tx_trace<T: Decompress>(
        &self,
        offset: u64,
        length: u32,
    ) -> Result<T, StaticFileError> {
        let bytes = self.transactions.tx_traces.read(offset, length)?;
        Ok(decompress_value(&bytes)?)
    }

    // Fixed-size reads use `key * record_size` as the offset. No MDBX pointer needed —
    // gated by the existence of a corresponding variable-size pointer entry.

    /// Read a block hash by block number. Returns `None` if not present.
    pub fn read_block_hash(
        &self,
        block_number: u64,
    ) -> Result<Option<katana_primitives::block::BlockHash>, StaticFileError> {
        match self.blocks.block_hashes.get(block_number)? {
            Some(bytes) => Ok(Some(katana_primitives::Felt::from_bytes_be_slice(&bytes))),
            None => Ok(None),
        }
    }

    /// Read a transaction hash by tx number. Returns `None` if not present.
    pub fn read_tx_hash(
        &self,
        tx_number: u64,
    ) -> Result<Option<katana_primitives::transaction::TxHash>, StaticFileError> {
        match self.transactions.tx_hashes.get(tx_number)? {
            Some(bytes) => Ok(Some(katana_primitives::Felt::from_bytes_be_slice(&bytes))),
            None => Ok(None),
        }
    }

    /// Read the block number for a transaction by tx number. Returns `None` if not present.
    pub fn read_tx_block(&self, tx_number: u64) -> Result<Option<u64>, StaticFileError> {
        match self.transactions.tx_blocks.get(tx_number)? {
            Some(bytes) => {
                let num = u64::from_be_bytes(bytes.as_slice().try_into().map_err(|_| {
                    StaticFileError::Codec(CodecError::Decode("invalid u64 bytes".into()))
                })?);
                Ok(Some(num))
            }
            None => Ok(None),
        }
    }

    // ---- Sync ----

    /// Fsync all static file columns to durable storage.
    /// Must be called BEFORE the MDBX transaction commits, so the data is
    /// guaranteed to be on disk when MDBX makes the pointers visible.
    pub fn sync(&self) -> Result<(), StaticFileError> {
        self.blocks.block_hashes.sync()?;
        self.blocks.headers.sync()?;
        self.blocks.block_body_indices.sync()?;
        self.blocks.block_state_updates.sync()?;

        self.transactions.tx_hashes.sync()?;
        self.transactions.tx_blocks.sync()?;
        self.transactions.transactions.sync()?;
        self.transactions.receipts.sync()?;
        self.transactions.tx_traces.sync()?;

        Ok(())
    }

    // ---- Buffer management ----

    /// Pre-allocate write buffers for an upcoming batch of blocks and transactions.
    ///
    /// `total_blocks` is the number of blocks, `total_txs` is the total number of
    /// transactions across all blocks. The sizes are estimates — the buffers will
    /// grow if needed, but pre-allocating avoids reallocations during the batch.
    pub fn reserve_for_batch(
        &self,
        total_blocks: usize,
        total_txs: usize,
    ) -> Result<(), StaticFileError> {
        // Estimates per entry (compressed sizes vary, these are conservative).
        const HEADER_EST: usize = 512;
        const STATE_UPDATE_EST: usize = 256;
        const TX_EST: usize = 256;
        const RECEIPT_EST: usize = 128;
        const TRACE_EST: usize = 128;

        self.blocks.headers.reserve(total_blocks * HEADER_EST)?;
        self.blocks.block_state_updates.reserve(total_blocks * STATE_UPDATE_EST)?;
        self.blocks.block_hashes.reserve(total_blocks * 32)?;

        self.transactions.transactions.reserve(total_txs * TX_EST)?;
        self.transactions.receipts.reserve(total_txs * RECEIPT_EST)?;
        self.transactions.tx_traces.reserve(total_txs * TRACE_EST)?;
        self.transactions.tx_hashes.reserve(total_txs * 32)?;
        self.transactions.tx_blocks.reserve(total_txs * 8)?;

        Ok(())
    }

    // ---- Remap ----

    /// Refresh memory maps to cover all data written so far.
    /// Call after a batch of writes to make new data visible via mmap reads.
    /// This is lightweight (no fsync) — just updates the mmap pointers.
    pub fn remap(&self) -> Result<(), StaticFileError> {
        self.blocks.block_hashes.remap()?;
        self.blocks.headers.remap()?;
        self.blocks.block_body_indices.remap()?;
        self.blocks.block_state_updates.remap()?;

        self.transactions.tx_hashes.remap()?;
        self.transactions.tx_blocks.remap()?;
        self.transactions.transactions.remap()?;
        self.transactions.receipts.remap()?;
        self.transactions.tx_traces.remap()?;

        Ok(())
    }

    // ---- Crash recovery ----

    /// Truncate block static files to match MDBX-committed state.
    ///
    /// - `block_count`: number of committed blocks (fixed-size columns truncate to this).
    /// - `headers_end`, `body_indices_end`, `state_updates_end`: byte position just past the last
    ///   committed entry in each variable-size column (i.e., `offset + length` from the last MDBX
    ///   pointer). Pass 0 if no entries exist.
    pub fn truncate_blocks(
        &self,
        block_count: u64,
        headers_end: u64,
        body_indices_end: u64,
        state_updates_end: u64,
    ) -> Result<(), StaticFileError> {
        self.blocks.block_hashes.truncate_to(block_count)?;
        self.blocks.headers.truncate(headers_end)?;
        self.blocks.block_body_indices.truncate(body_indices_end)?;
        self.blocks.block_state_updates.truncate(state_updates_end)?;
        Ok(())
    }

    /// Truncate transaction static files to match MDBX-committed state.
    /// See [`truncate_blocks`](Self::truncate_blocks) for parameter semantics.
    pub fn truncate_transactions(
        &self,
        tx_count: u64,
        transactions_end: u64,
        receipts_end: u64,
        traces_end: u64,
    ) -> Result<(), StaticFileError> {
        self.transactions.tx_hashes.truncate_to(tx_count)?;
        self.transactions.tx_blocks.truncate_to(tx_count)?;
        self.transactions.transactions.truncate(transactions_end)?;
        self.transactions.receipts.truncate(receipts_end)?;
        self.transactions.tx_traces.truncate(traces_end)?;
        Ok(())
    }
}

/// Errors that can occur during static file operations.
#[derive(Debug, thiserror::Error)]
pub enum StaticFileError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("codec error: {0}")]
    Codec(#[from] CodecError),
}

#[cfg(test)]
mod tests {
    use katana_primitives::felt;
    use katana_primitives::state::StateUpdates;

    use super::*;
    use crate::models::block::StoredBlockBodyIndices;
    use crate::models::state_update::StateUpdateEnvelope;
    use crate::models::{ReceiptEnvelope, TxEnvelope, VersionedHeader, VersionedTx};

    #[test]
    fn roundtrip_block_data() {
        let sf = StaticFiles::<AnyStore>::in_memory();

        let header = VersionedHeader::default();
        let block_hash = felt!("0xdeadbeef");
        let body_indices = StoredBlockBodyIndices { tx_offset: 0, tx_count: 1 };
        let state_updates = StateUpdateEnvelope::from(StateUpdates::default());

        // Write — returns pointers for MDBX.
        let (h_off, h_len) = sf.append_header(header.clone()).unwrap();
        sf.append_block_hash(0, block_hash).unwrap();
        let (bi_off, bi_len) = sf.append_block_body_indices(body_indices.clone()).unwrap();
        let (su_off, su_len) = sf.append_block_state_update(state_updates.clone()).unwrap();

        // Read using pointers.
        let h: VersionedHeader = sf.read_header(h_off, h_len).unwrap();
        assert_eq!(h, header);

        let bh = sf.read_block_hash(0).unwrap().unwrap();
        assert_eq!(bh, block_hash);

        let bi: StoredBlockBodyIndices = sf.read_block_body_indices(bi_off, bi_len).unwrap();
        assert_eq!(bi, body_indices);

        let su: StateUpdateEnvelope = sf.read_block_state_update(su_off, su_len).unwrap();
        assert_eq!(su, state_updates);
    }

    #[test]
    fn roundtrip_transaction_data() {
        use katana_primitives::execution::TypedTransactionExecutionInfo;
        use katana_primitives::receipt::{InvokeTxReceipt, Receipt};
        use katana_primitives::transaction::{InvokeTx, Tx};

        let sf = StaticFiles::<AnyStore>::in_memory();

        let tx_hash = felt!("0x1234");
        let tx_envelope =
            TxEnvelope::from(VersionedTx::from(Tx::Invoke(InvokeTx::V1(Default::default()))));
        let receipt_envelope = ReceiptEnvelope::from(Receipt::Invoke(InvokeTxReceipt {
            revert_error: None,
            events: Vec::new(),
            fee: Default::default(),
            messages_sent: Vec::new(),
            execution_resources: Default::default(),
        }));
        let trace = TypedTransactionExecutionInfo::default();

        // Write — returns pointers for MDBX.
        let (tx_off, tx_len) = sf.append_transaction(tx_envelope.clone()).unwrap();
        sf.append_tx_hash(0, tx_hash).unwrap();
        sf.append_tx_block(0, 0).unwrap();
        let (r_off, r_len) = sf.append_receipt(receipt_envelope.clone()).unwrap();
        let (tr_off, tr_len) = sf.append_tx_trace(trace.clone()).unwrap();

        // Read using pointers.
        let t: TxEnvelope = sf.read_transaction(tx_off, tx_len).unwrap();
        assert_eq!(t, tx_envelope);

        let h = sf.read_tx_hash(0).unwrap().unwrap();
        assert_eq!(h, tx_hash);

        let b = sf.read_tx_block(0).unwrap().unwrap();
        assert_eq!(b, 0);

        let r: ReceiptEnvelope = sf.read_receipt(r_off, r_len).unwrap();
        assert_eq!(r, receipt_envelope);

        let tr: TypedTransactionExecutionInfo = sf.read_tx_trace(tr_off, tr_len).unwrap();
        assert_eq!(tr, trace);
    }
}
