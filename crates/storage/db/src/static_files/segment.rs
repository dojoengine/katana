use std::io;
use std::path::Path;

use katana_primitives::block::{BlockHash, BlockNumber};
use katana_primitives::execution::TypedTransactionExecutionInfo;
use katana_primitives::transaction::TxNumber;

use super::column::{FixedColumn, IndexedColumn};
use super::manifest::Manifest;
use super::store::{AnyStore, FileStore, MemoryStore, StaticStore};
use crate::codecs::{Compress, Decompress};
use crate::error::CodecError;
use crate::models::block::StoredBlockBodyIndices;
use crate::models::state_update::StateUpdateEnvelope;
use crate::models::{ReceiptEnvelope, TxEnvelope, VersionedHeader};

/// Block-indexed segment grouping block-level static columns.
pub struct BlockSegment<S: StaticStore> {
    pub headers: IndexedColumn<S>,
    pub block_hashes: FixedColumn<S>,
    pub block_body_indices: IndexedColumn<S>,
    pub block_state_updates: IndexedColumn<S>,
}

/// Transaction-indexed segment grouping transaction-level static columns.
pub struct TxSegment<S: StaticStore> {
    pub transactions: IndexedColumn<S>,
    pub receipts: IndexedColumn<S>,
    pub tx_hashes: FixedColumn<S>,
    pub tx_blocks: FixedColumn<S>,
    pub tx_traces: IndexedColumn<S>,
}

/// Top-level container for all static file data.
pub struct StaticFiles<S: StaticStore> {
    pub blocks: BlockSegment<S>,
    pub transactions: TxSegment<S>,
    manifest: parking_lot::Mutex<Manifest>,
    manifest_path: Option<std::path::PathBuf>,
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

        let manifest_path = base_path.join("manifest.json");
        let manifest = Manifest::read_from_file(&manifest_path)?.unwrap_or_default();

        let open = |dir: &Path, name: &str| -> io::Result<AnyStore> {
            Ok(AnyStore::File(FileStore::open(&dir.join(name))?))
        };

        let blocks = BlockSegment {
            headers: IndexedColumn::new(
                open(&blocks_path, "headers.dat")?,
                open(&blocks_path, "headers.idx")?,
            ),
            block_hashes: FixedColumn::new(open(&blocks_path, "block_hashes.dat")?, 32),
            block_body_indices: IndexedColumn::new(
                open(&blocks_path, "block_body_indices.dat")?,
                open(&blocks_path, "block_body_indices.idx")?,
            ),
            block_state_updates: IndexedColumn::new(
                open(&blocks_path, "block_state_updates.dat")?,
                open(&blocks_path, "block_state_updates.idx")?,
            ),
        };

        let transactions = TxSegment {
            transactions: IndexedColumn::new(
                open(&txs_path, "transactions.dat")?,
                open(&txs_path, "transactions.idx")?,
            ),
            receipts: IndexedColumn::new(
                open(&txs_path, "receipts.dat")?,
                open(&txs_path, "receipts.idx")?,
            ),
            tx_hashes: FixedColumn::new(open(&txs_path, "tx_hashes.dat")?, 32),
            tx_blocks: FixedColumn::new(open(&txs_path, "tx_blocks.dat")?, 8),
            tx_traces: IndexedColumn::new(
                open(&txs_path, "tx_traces.dat")?,
                open(&txs_path, "tx_traces.idx")?,
            ),
        };

        let sf = Self {
            blocks,
            transactions,
            manifest: parking_lot::Mutex::new(manifest.clone()),
            manifest_path: Some(manifest_path),
        };

        // Crash recovery: truncate columns to manifest counts.
        sf.recover(&manifest)?;

        Ok(sf)
    }

    /// Create in-memory static files (tests, ephemeral mode).
    pub fn in_memory() -> Self {
        let mem = || AnyStore::Memory(MemoryStore::new());

        let blocks = BlockSegment {
            headers: IndexedColumn::new(mem(), mem()),
            block_hashes: FixedColumn::new(mem(), 32),
            block_body_indices: IndexedColumn::new(mem(), mem()),
            block_state_updates: IndexedColumn::new(mem(), mem()),
        };

        let transactions = TxSegment {
            transactions: IndexedColumn::new(mem(), mem()),
            receipts: IndexedColumn::new(mem(), mem()),
            tx_hashes: FixedColumn::new(mem(), 32),
            tx_blocks: FixedColumn::new(mem(), 8),
            tx_traces: IndexedColumn::new(mem(), mem()),
        };

        Self {
            blocks,
            transactions,
            manifest: parking_lot::Mutex::new(Manifest::default()),
            manifest_path: None,
        }
    }
}

// -- Crash recovery --

impl<S: StaticStore> StaticFiles<S> {
    fn recover(&self, manifest: &Manifest) -> io::Result<()> {
        let bc = manifest.latest_block_count;
        self.blocks.headers.truncate_to(bc)?;
        self.blocks.block_hashes.truncate_to(bc)?;
        self.blocks.block_body_indices.truncate_to(bc)?;
        self.blocks.block_state_updates.truncate_to(bc)?;

        let tc = manifest.latest_tx_count;
        self.transactions.transactions.truncate_to(tc)?;
        self.transactions.receipts.truncate_to(tc)?;
        self.transactions.tx_hashes.truncate_to(tc)?;
        self.transactions.tx_blocks.truncate_to(tc)?;
        self.transactions.tx_traces.truncate_to(tc)?;

        Ok(())
    }
}

// -- Typed read/write API --

/// Helper to compress a value using the existing Compress trait.
fn compress_value<T: Compress>(value: T) -> Result<Vec<u8>, CodecError> {
    let compressed = value.compress()?;
    Ok(compressed.into())
}

/// Helper to decompress a value using the existing Decompress trait.
fn decompress_value<T: Decompress>(bytes: &[u8]) -> Result<T, CodecError> {
    T::decompress(bytes)
}

impl<S: StaticStore> StaticFiles<S> {
    // ---- Block reads ----

    pub fn header(&self, num: BlockNumber) -> Result<Option<VersionedHeader>, StaticFileError> {
        match self.blocks.headers.get(num)? {
            Some(bytes) => Ok(Some(decompress_value(&bytes)?)),
            None => Ok(None),
        }
    }

    pub fn block_hash(&self, num: BlockNumber) -> Result<Option<BlockHash>, StaticFileError> {
        match self.blocks.block_hashes.get(num)? {
            Some(bytes) => {
                let hash = katana_primitives::Felt::from_bytes_be_slice(&bytes);
                Ok(Some(hash))
            }
            None => Ok(None),
        }
    }

    pub fn block_body_indices(
        &self,
        num: BlockNumber,
    ) -> Result<Option<StoredBlockBodyIndices>, StaticFileError> {
        match self.blocks.block_body_indices.get(num)? {
            Some(bytes) => Ok(Some(decompress_value(&bytes)?)),
            None => Ok(None),
        }
    }

    pub fn block_state_update(
        &self,
        num: BlockNumber,
    ) -> Result<Option<StateUpdateEnvelope>, StaticFileError> {
        match self.blocks.block_state_updates.get(num)? {
            Some(bytes) => Ok(Some(decompress_value(&bytes)?)),
            None => Ok(None),
        }
    }

    // ---- Transaction reads ----

    pub fn transaction(&self, num: TxNumber) -> Result<Option<TxEnvelope>, StaticFileError> {
        match self.transactions.transactions.get(num)? {
            Some(bytes) => Ok(Some(decompress_value(&bytes)?)),
            None => Ok(None),
        }
    }

    pub fn receipt(&self, num: TxNumber) -> Result<Option<ReceiptEnvelope>, StaticFileError> {
        match self.transactions.receipts.get(num)? {
            Some(bytes) => Ok(Some(decompress_value(&bytes)?)),
            None => Ok(None),
        }
    }

    pub fn tx_hash(
        &self,
        num: TxNumber,
    ) -> Result<Option<katana_primitives::transaction::TxHash>, StaticFileError> {
        match self.transactions.tx_hashes.get(num)? {
            Some(bytes) => {
                let hash = katana_primitives::Felt::from_bytes_be_slice(&bytes);
                Ok(Some(hash))
            }
            None => Ok(None),
        }
    }

    pub fn tx_block(&self, num: TxNumber) -> Result<Option<BlockNumber>, StaticFileError> {
        match self.transactions.tx_blocks.get(num)? {
            Some(bytes) => {
                let block_num = u64::from_be_bytes(bytes.as_slice().try_into().map_err(|_| {
                    StaticFileError::Codec(CodecError::Decode("invalid u64 bytes".into()))
                })?);
                Ok(Some(block_num))
            }
            None => Ok(None),
        }
    }

    pub fn tx_trace(
        &self,
        num: TxNumber,
    ) -> Result<Option<TypedTransactionExecutionInfo>, StaticFileError> {
        match self.transactions.tx_traces.get(num)? {
            Some(bytes) => Ok(Some(decompress_value(&bytes)?)),
            None => Ok(None),
        }
    }

    // ---- Metadata ----

    pub fn latest_block_number(&self) -> Result<Option<BlockNumber>, StaticFileError> {
        let manifest = self.manifest.lock();
        Ok(manifest.latest_block_number())
    }

    pub fn total_transactions(&self) -> Result<u64, StaticFileError> {
        let manifest = self.manifest.lock();
        Ok(manifest.latest_tx_count)
    }

    // ---- Block writes ----

    pub fn append_block(
        &self,
        block_number: BlockNumber,
        header: VersionedHeader,
        block_hash: BlockHash,
        body_indices: StoredBlockBodyIndices,
        state_updates: StateUpdateEnvelope,
    ) -> Result<(), StaticFileError> {
        let header_bytes = compress_value(header)?;
        self.blocks.headers.append(block_number, &header_bytes)?;

        let hash_bytes = block_hash.to_bytes_be();
        self.blocks.block_hashes.append(block_number, &hash_bytes)?;

        let indices_bytes = compress_value(body_indices)?;
        self.blocks.block_body_indices.append(block_number, &indices_bytes)?;

        let state_bytes = compress_value(state_updates)?;
        self.blocks.block_state_updates.append(block_number, &state_bytes)?;

        Ok(())
    }

    // ---- Transaction writes ----

    pub fn append_transaction(
        &self,
        tx_number: TxNumber,
        transaction: TxEnvelope,
        tx_hash: katana_primitives::transaction::TxHash,
        block_number: BlockNumber,
        receipt: ReceiptEnvelope,
        trace: TypedTransactionExecutionInfo,
    ) -> Result<(), StaticFileError> {
        let tx_bytes = compress_value(transaction)?;
        self.transactions.transactions.append(tx_number, &tx_bytes)?;

        let hash_bytes = tx_hash.to_bytes_be();
        self.transactions.tx_hashes.append(tx_number, &hash_bytes)?;

        let block_bytes = block_number.to_be_bytes();
        self.transactions.tx_blocks.append(tx_number, &block_bytes)?;

        let receipt_bytes = compress_value(receipt)?;
        self.transactions.receipts.append(tx_number, &receipt_bytes)?;

        let trace_bytes = compress_value(trace)?;
        self.transactions.tx_traces.append(tx_number, &trace_bytes)?;

        Ok(())
    }

    // ---- Commit ----

    /// Fsync all columns and update the manifest with new counts.
    pub fn commit(&self, block_count: u64, tx_count: u64) -> Result<(), StaticFileError> {
        // Sync all block columns.
        self.blocks.headers.sync()?;
        self.blocks.block_hashes.sync()?;
        self.blocks.block_body_indices.sync()?;
        self.blocks.block_state_updates.sync()?;

        // Sync all transaction columns.
        self.transactions.transactions.sync()?;
        self.transactions.receipts.sync()?;
        self.transactions.tx_hashes.sync()?;
        self.transactions.tx_blocks.sync()?;
        self.transactions.tx_traces.sync()?;

        // Update and write manifest.
        let mut manifest = self.manifest.lock();
        manifest.latest_block_count = block_count;
        manifest.latest_tx_count = tx_count;

        if let Some(ref path) = self.manifest_path {
            manifest.write_to_file(path)?;
        }

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

    #[test]
    fn roundtrip_block_data() {
        let sf = StaticFiles::<AnyStore>::in_memory();

        let header = VersionedHeader::default();
        let block_hash: BlockHash = felt!("0xdeadbeef");
        let body_indices = StoredBlockBodyIndices { tx_offset: 0, tx_count: 1 };
        let state_updates = StateUpdateEnvelope::from(StateUpdates::default());

        sf.append_block(0, header.clone(), block_hash, body_indices.clone(), state_updates.clone())
            .unwrap();

        sf.commit(1, 0).unwrap();

        assert_eq!(sf.latest_block_number().unwrap(), Some(0));

        let h = sf.header(0).unwrap().unwrap();
        assert_eq!(h, header);

        let bh = sf.block_hash(0).unwrap().unwrap();
        assert_eq!(bh, block_hash);

        let bi = sf.block_body_indices(0).unwrap().unwrap();
        assert_eq!(bi, body_indices);

        let su = sf.block_state_update(0).unwrap().unwrap();
        assert_eq!(su, state_updates);

        // Key beyond range returns None.
        assert!(sf.header(1).unwrap().is_none());
    }

    #[test]
    fn roundtrip_transaction_data() {
        use katana_primitives::execution::TypedTransactionExecutionInfo;
        use katana_primitives::receipt::{InvokeTxReceipt, Receipt};
        use katana_primitives::transaction::{InvokeTx, Tx};

        use crate::models::VersionedTx;

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

        sf.append_transaction(
            0,
            tx_envelope.clone(),
            tx_hash,
            0,
            receipt_envelope.clone(),
            trace.clone(),
        )
        .unwrap();

        sf.commit(1, 1).unwrap();

        assert_eq!(sf.total_transactions().unwrap(), 1);

        let t = sf.transaction(0).unwrap().unwrap();
        assert_eq!(t, tx_envelope);

        let h = sf.tx_hash(0).unwrap().unwrap();
        assert_eq!(h, tx_hash);

        let b = sf.tx_block(0).unwrap().unwrap();
        assert_eq!(b, 0);

        let r = sf.receipt(0).unwrap().unwrap();
        assert_eq!(r, receipt_envelope);

        let tr = sf.tx_trace(0).unwrap().unwrap();
        assert_eq!(tr, trace);

        assert!(sf.transaction(1).unwrap().is_none());
    }
}
