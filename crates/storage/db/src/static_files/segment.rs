use std::io;
use std::path::Path;

use super::column::{DataColumn, FixedColumn};
use super::store::{AnyStore, FileStore, MemoryStore, StaticStore};
use crate::codecs::{Compress, Decompress};
use crate::error::CodecError;

/// Block-indexed segment grouping block-level static columns.
pub struct BlockSegment<S: StaticStore> {
    /// Fixed 32B per block — read by key, gated by Headers pointer in MDBX.
    pub block_hashes: FixedColumn<S>,
    /// Variable-size — (offset, length) stored in MDBX as StaticFileRef.
    pub headers: DataColumn<S>,
    /// Variable-size — (offset, length) stored in MDBX as StaticFileRef.
    pub block_body_indices: DataColumn<S>,
    /// Variable-size — (offset, length) stored in MDBX as StaticFileRef.
    pub block_state_updates: DataColumn<S>,
}

/// Transaction-indexed segment grouping transaction-level static columns.
pub struct TxSegment<S: StaticStore> {
    /// Fixed 32B per tx — read by key, gated by Transactions pointer in MDBX.
    pub tx_hashes: FixedColumn<S>,
    /// Fixed 8B per tx — read by key, gated by Transactions pointer in MDBX.
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
pub struct StaticFiles<S: StaticStore> {
    pub blocks: BlockSegment<S>,
    pub transactions: TxSegment<S>,
}

impl<S: StaticStore> std::fmt::Debug for StaticFiles<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StaticFiles").finish_non_exhaustive()
    }
}

// -- Constructors --

impl StaticFiles<AnyStore> {
    /// Open file-backed static files at the given directory (production).
    pub fn open_file(base_path: &Path) -> io::Result<Self> {
        let blocks_path = base_path.join("blocks");
        let txs_path = base_path.join("transactions");

        std::fs::create_dir_all(&blocks_path)?;
        std::fs::create_dir_all(&txs_path)?;

        let open = |dir: &Path, name: &str| -> io::Result<AnyStore> {
            Ok(AnyStore::File(FileStore::open(&dir.join(name))?))
        };

        let blocks = BlockSegment {
            block_hashes: FixedColumn::new(open(&blocks_path, "block_hashes.dat")?, 32),
            headers: DataColumn::new(open(&blocks_path, "headers.dat")?),
            block_body_indices: DataColumn::new(open(&blocks_path, "block_body_indices.dat")?),
            block_state_updates: DataColumn::new(open(&blocks_path, "block_state_updates.dat")?),
        };

        let transactions = TxSegment {
            tx_hashes: FixedColumn::new(open(&txs_path, "tx_hashes.dat")?, 32),
            tx_blocks: FixedColumn::new(open(&txs_path, "tx_blocks.dat")?, 8),
            transactions: DataColumn::new(open(&txs_path, "transactions.dat")?),
            receipts: DataColumn::new(open(&txs_path, "receipts.dat")?),
            tx_traces: DataColumn::new(open(&txs_path, "tx_traces.dat")?),
        };

        Ok(Self { blocks, transactions })
    }

    /// Create in-memory static files (tests, ephemeral mode).
    pub fn in_memory() -> Self {
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
    // ---- Block-level variable-size writes (return offset+length) ----

    pub fn append_header<T: Compress>(&self, header: T) -> Result<(u64, u32), StaticFileError> {
        let bytes = compress_value(header)?;
        Ok(self.blocks.headers.append(&bytes)?)
    }

    pub fn append_block_body_indices<T: Compress>(
        &self,
        indices: T,
    ) -> Result<(u64, u32), StaticFileError> {
        let bytes = compress_value(indices)?;
        Ok(self.blocks.block_body_indices.append(&bytes)?)
    }

    pub fn append_block_state_update<T: Compress>(
        &self,
        update: T,
    ) -> Result<(u64, u32), StaticFileError> {
        let bytes = compress_value(update)?;
        Ok(self.blocks.block_state_updates.append(&bytes)?)
    }

    // ---- Block-level fixed-size writes ----

    pub fn append_block_hash(
        &self,
        block_number: u64,
        hash: katana_primitives::block::BlockHash,
    ) -> Result<(), StaticFileError> {
        self.blocks.block_hashes.append(block_number, &hash.to_bytes_be())?;
        Ok(())
    }

    // ---- Transaction-level variable-size writes (return offset+length) ----

    pub fn append_transaction<T: Compress>(&self, tx: T) -> Result<(u64, u32), StaticFileError> {
        let bytes = compress_value(tx)?;
        Ok(self.transactions.transactions.append(&bytes)?)
    }

    pub fn append_receipt<T: Compress>(&self, receipt: T) -> Result<(u64, u32), StaticFileError> {
        let bytes = compress_value(receipt)?;
        Ok(self.transactions.receipts.append(&bytes)?)
    }

    pub fn append_tx_trace<T: Compress>(&self, trace: T) -> Result<(u64, u32), StaticFileError> {
        let bytes = compress_value(trace)?;
        Ok(self.transactions.tx_traces.append(&bytes)?)
    }

    // ---- Transaction-level fixed-size writes ----

    pub fn append_tx_hash(
        &self,
        tx_number: u64,
        hash: katana_primitives::transaction::TxHash,
    ) -> Result<(), StaticFileError> {
        self.transactions.tx_hashes.append(tx_number, &hash.to_bytes_be())?;
        Ok(())
    }

    pub fn append_tx_block(
        &self,
        tx_number: u64,
        block_number: u64,
    ) -> Result<(), StaticFileError> {
        self.transactions.tx_blocks.append(tx_number, &block_number.to_be_bytes())?;
        Ok(())
    }

    // ---- Variable-size reads (caller provides offset+length from MDBX) ----

    pub fn read_header<T: Decompress>(
        &self,
        offset: u64,
        length: u32,
    ) -> Result<T, StaticFileError> {
        let bytes = self.blocks.headers.read(offset, length)?;
        Ok(decompress_value(&bytes)?)
    }

    pub fn read_block_body_indices<T: Decompress>(
        &self,
        offset: u64,
        length: u32,
    ) -> Result<T, StaticFileError> {
        let bytes = self.blocks.block_body_indices.read(offset, length)?;
        Ok(decompress_value(&bytes)?)
    }

    pub fn read_block_state_update<T: Decompress>(
        &self,
        offset: u64,
        length: u32,
    ) -> Result<T, StaticFileError> {
        let bytes = self.blocks.block_state_updates.read(offset, length)?;
        Ok(decompress_value(&bytes)?)
    }

    pub fn read_transaction<T: Decompress>(
        &self,
        offset: u64,
        length: u32,
    ) -> Result<T, StaticFileError> {
        let bytes = self.transactions.transactions.read(offset, length)?;
        Ok(decompress_value(&bytes)?)
    }

    pub fn read_receipt<T: Decompress>(
        &self,
        offset: u64,
        length: u32,
    ) -> Result<T, StaticFileError> {
        let bytes = self.transactions.receipts.read(offset, length)?;
        Ok(decompress_value(&bytes)?)
    }

    pub fn read_tx_trace<T: Decompress>(
        &self,
        offset: u64,
        length: u32,
    ) -> Result<T, StaticFileError> {
        let bytes = self.transactions.tx_traces.read(offset, length)?;
        Ok(decompress_value(&bytes)?)
    }

    // ---- Fixed-size reads ----

    pub fn read_block_hash(
        &self,
        block_number: u64,
    ) -> Result<Option<katana_primitives::block::BlockHash>, StaticFileError> {
        match self.blocks.block_hashes.get(block_number)? {
            Some(bytes) => Ok(Some(katana_primitives::Felt::from_bytes_be_slice(&bytes))),
            None => Ok(None),
        }
    }

    pub fn read_tx_hash(
        &self,
        tx_number: u64,
    ) -> Result<Option<katana_primitives::transaction::TxHash>, StaticFileError> {
        match self.transactions.tx_hashes.get(tx_number)? {
            Some(bytes) => Ok(Some(katana_primitives::Felt::from_bytes_be_slice(&bytes))),
            None => Ok(None),
        }
    }

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

    /// Truncate static files to match MDBX-committed state.
    ///
    /// For variable-size columns, `last_ptr_byte_end` is the end of the last
    /// committed entry (offset + length from the last MDBX pointer). Pass 0
    /// if no entries exist.
    ///
    /// For fixed-size columns, truncate to `count` entries.
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
