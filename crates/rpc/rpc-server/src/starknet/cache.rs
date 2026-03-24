use std::sync::Arc;

use katana_primitives::block::BlockNumber;
use katana_primitives::class::ClassHash;
use katana_primitives::transaction::TxHash;
use katana_rpc_types::block::{BlockWithReceipts, BlockWithTxHashes, BlockWithTxs};
use katana_rpc_types::class::Class;
use katana_rpc_types::receipt::TxReceiptWithBlockInfo;
use katana_rpc_types::state_update::ConfirmedStateUpdate;
use katana_rpc_types::trace::{TxTrace, TxTraceWithHash};
use katana_rpc_types::transaction::RpcTxWithHash;
use quick_cache::sync::Cache;

/// Default maximum number of cached blocks.
const DEFAULT_CACHE_MAX_BLOCKS: usize = 128;
/// Default maximum number of cached transactions.
const DEFAULT_CACHE_MAX_TRANSACTIONS: usize = 1024;
/// Default maximum number of cached classes.
const DEFAULT_CACHE_MAX_CLASSES: usize = 256;

/// RPC response cache that stores already-converted RPC types.
///
/// All cached data is for confirmed (immutable) blocks only. Pending/pre-confirmed
/// data is never cached. Cache keys are normalized: block-keyed data uses [`BlockNumber`],
/// transaction-keyed data uses [`TxHash`], and class-keyed data uses
/// `(ClassHash, BlockNumber)`.
///
/// When a cache is created with capacity 0, inserts are immediately evicted (no-op).
#[derive(Debug, Clone)]
pub struct RpcCache {
    inner: Arc<RpcCacheInner>,
}

struct RpcCacheInner {
    blocks_with_txs: Cache<BlockNumber, BlockWithTxs>,
    blocks_with_tx_hashes: Cache<BlockNumber, BlockWithTxHashes>,
    blocks_with_receipts: Cache<BlockNumber, BlockWithReceipts>,
    transactions: Cache<TxHash, RpcTxWithHash>,
    receipts: Cache<TxHash, TxReceiptWithBlockInfo>,
    classes: Cache<(ClassHash, BlockNumber), Class>,
    state_updates: Cache<BlockNumber, ConfirmedStateUpdate>,
    traces: Cache<TxHash, TxTrace>,
    block_traces: Cache<BlockNumber, Vec<TxTraceWithHash>>,
}

impl std::fmt::Debug for RpcCacheInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RpcCacheInner").finish_non_exhaustive()
    }
}

impl RpcCache {
    /// Creates a new [`RpcCache`] from the given configuration.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RpcCacheInner {
                blocks_with_txs: Cache::new(DEFAULT_CACHE_MAX_BLOCKS),
                blocks_with_tx_hashes: Cache::new(DEFAULT_CACHE_MAX_BLOCKS),
                blocks_with_receipts: Cache::new(DEFAULT_CACHE_MAX_BLOCKS),
                transactions: Cache::new(DEFAULT_CACHE_MAX_TRANSACTIONS),
                receipts: Cache::new(DEFAULT_CACHE_MAX_TRANSACTIONS),
                classes: Cache::new(DEFAULT_CACHE_MAX_CLASSES),
                state_updates: Cache::new(DEFAULT_CACHE_MAX_BLOCKS),
                traces: Cache::new(DEFAULT_CACHE_MAX_TRANSACTIONS),
                block_traces: Cache::new(DEFAULT_CACHE_MAX_BLOCKS),
            }),
        }
    }

    // --- Blocks with transactions ---

    pub fn get_block_with_txs(&self, block_num: BlockNumber) -> Option<BlockWithTxs> {
        self.inner.blocks_with_txs.get(&block_num)
    }

    pub fn insert_block_with_txs(&self, block_num: BlockNumber, block: BlockWithTxs) {
        self.inner.blocks_with_txs.insert(block_num, block);
    }

    // --- Blocks with transaction hashes ---

    pub fn get_block_with_tx_hashes(&self, block_num: BlockNumber) -> Option<BlockWithTxHashes> {
        self.inner.blocks_with_tx_hashes.get(&block_num)
    }

    pub fn insert_block_with_tx_hashes(&self, block_num: BlockNumber, block: BlockWithTxHashes) {
        self.inner.blocks_with_tx_hashes.insert(block_num, block);
    }

    // --- Blocks with receipts ---

    pub fn get_block_with_receipts(&self, block_num: BlockNumber) -> Option<BlockWithReceipts> {
        self.inner.blocks_with_receipts.get(&block_num)
    }

    pub fn insert_block_with_receipts(&self, block_num: BlockNumber, block: BlockWithReceipts) {
        self.inner.blocks_with_receipts.insert(block_num, block);
    }

    // --- Transactions ---

    pub fn get_transaction(&self, hash: TxHash) -> Option<RpcTxWithHash> {
        self.inner.transactions.get(&hash)
    }

    pub fn insert_transaction(&self, hash: TxHash, tx: RpcTxWithHash) {
        self.inner.transactions.insert(hash, tx);
    }

    // --- Receipts ---

    pub fn get_receipt(&self, hash: TxHash) -> Option<TxReceiptWithBlockInfo> {
        self.inner.receipts.get(&hash)
    }

    pub fn insert_receipt(&self, hash: TxHash, receipt: TxReceiptWithBlockInfo) {
        self.inner.receipts.insert(hash, receipt);
    }

    // --- Classes ---

    pub fn get_class(&self, key: (ClassHash, BlockNumber)) -> Option<Class> {
        self.inner.classes.get(&key)
    }

    pub fn insert_class(&self, key: (ClassHash, BlockNumber), class: Class) {
        self.inner.classes.insert(key, class);
    }

    // --- State updates ---

    pub fn get_state_update(&self, block_num: BlockNumber) -> Option<ConfirmedStateUpdate> {
        self.inner.state_updates.get(&block_num)
    }

    pub fn insert_state_update(&self, block_num: BlockNumber, update: ConfirmedStateUpdate) {
        self.inner.state_updates.insert(block_num, update);
    }

    // --- Transaction traces ---

    pub fn get_trace(&self, hash: TxHash) -> Option<TxTrace> {
        self.inner.traces.get(&hash)
    }

    pub fn insert_trace(&self, hash: TxHash, trace: TxTrace) {
        self.inner.traces.insert(hash, trace);
    }

    // --- Block traces ---

    pub fn get_block_traces(&self, block_num: BlockNumber) -> Option<Vec<TxTraceWithHash>> {
        self.inner.block_traces.get(&block_num)
    }

    pub fn insert_block_traces(&self, block_num: BlockNumber, traces: Vec<TxTraceWithHash>) {
        self.inner.block_traces.insert(block_num, traces);
    }
}

impl Default for RpcCache {
    fn default() -> Self {
        Self::new()
    }
}
