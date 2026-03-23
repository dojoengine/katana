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
pub const DEFAULT_CACHE_MAX_BLOCKS: usize = 128;
/// Default maximum number of cached transactions.
pub const DEFAULT_CACHE_MAX_TRANSACTIONS: usize = 1024;
/// Default maximum number of cached classes.
pub const DEFAULT_CACHE_MAX_CLASSES: usize = 256;

/// Configuration for the RPC response cache.
///
/// Each field controls the maximum number of entries for that cache type.
/// Set a field to 0 to disable that specific cache.
#[derive(Debug, Clone)]
pub struct RpcCacheConfig {
    /// Maximum number of cached block-with-txs entries.
    pub max_blocks_with_txs: usize,
    /// Maximum number of cached block-with-tx-hashes entries.
    pub max_blocks_with_tx_hashes: usize,
    /// Maximum number of cached block-with-receipts entries.
    pub max_blocks_with_receipts: usize,
    /// Maximum number of cached transaction entries.
    pub max_transactions: usize,
    /// Maximum number of cached receipt entries.
    pub max_receipts: usize,
    /// Maximum number of cached contract class entries.
    pub max_classes: usize,
    /// Maximum number of cached state update entries.
    pub max_state_updates: usize,
    /// Maximum number of cached single transaction trace entries.
    pub max_traces: usize,
    /// Maximum number of cached block trace entries.
    pub max_block_traces: usize,
}

impl RpcCacheConfig {
    /// Creates a config from the 3 grouped CLI values.
    ///
    /// - `max_blocks` controls block-keyed caches (with_txs, with_tx_hashes, with_receipts,
    ///   state_updates, block_traces). Receipts and block traces get half the capacity.
    /// - `max_transactions` controls tx-keyed caches (transactions, receipts, traces). Traces get
    ///   half the capacity.
    /// - `max_classes` controls the class cache directly.
    pub fn from_cli(max_blocks: usize, max_transactions: usize, max_classes: usize) -> Self {
        Self {
            max_blocks_with_txs: max_blocks,
            max_blocks_with_tx_hashes: max_blocks,
            max_blocks_with_receipts: max_blocks / 2,
            max_state_updates: max_blocks,
            max_block_traces: max_blocks / 2,
            max_transactions,
            max_receipts: max_transactions,
            max_traces: max_transactions / 2,
            max_classes,
        }
    }
}

impl Default for RpcCacheConfig {
    fn default() -> Self {
        Self::from_cli(
            DEFAULT_CACHE_MAX_BLOCKS,
            DEFAULT_CACHE_MAX_TRANSACTIONS,
            DEFAULT_CACHE_MAX_CLASSES,
        )
    }
}

/// RPC response cache that stores already-converted RPC types.
///
/// All cached data is for confirmed (immutable) blocks only. Pending/pre-confirmed
/// data is never cached. Cache keys are normalized: block-keyed data uses [`BlockNumber`],
/// transaction-keyed data uses [`TxHash`], and class-keyed data uses
/// `(ClassHash, BlockNumber)`.
#[derive(Debug, Clone)]
pub struct RpcCache {
    inner: Arc<RpcCacheInner>,
}

struct RpcCacheInner {
    blocks_with_txs: Option<Cache<BlockNumber, BlockWithTxs>>,
    blocks_with_tx_hashes: Option<Cache<BlockNumber, BlockWithTxHashes>>,
    blocks_with_receipts: Option<Cache<BlockNumber, BlockWithReceipts>>,
    transactions: Option<Cache<TxHash, RpcTxWithHash>>,
    receipts: Option<Cache<TxHash, TxReceiptWithBlockInfo>>,
    classes: Option<Cache<(ClassHash, BlockNumber), Class>>,
    state_updates: Option<Cache<BlockNumber, ConfirmedStateUpdate>>,
    traces: Option<Cache<TxHash, TxTrace>>,
    block_traces: Option<Cache<BlockNumber, Vec<TxTraceWithHash>>>,
}

impl std::fmt::Debug for RpcCacheInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RpcCacheInner")
            .field("blocks_with_txs", &self.blocks_with_txs.as_ref().map(|_| ".."))
            .field("blocks_with_tx_hashes", &self.blocks_with_tx_hashes.as_ref().map(|_| ".."))
            .field("blocks_with_receipts", &self.blocks_with_receipts.as_ref().map(|_| ".."))
            .field("transactions", &self.transactions.as_ref().map(|_| ".."))
            .field("receipts", &self.receipts.as_ref().map(|_| ".."))
            .field("classes", &self.classes.as_ref().map(|_| ".."))
            .field("state_updates", &self.state_updates.as_ref().map(|_| ".."))
            .field("traces", &self.traces.as_ref().map(|_| ".."))
            .field("block_traces", &self.block_traces.as_ref().map(|_| ".."))
            .finish()
    }
}

fn make_cache<K, V>(size: usize) -> Option<Cache<K, V>>
where
    K: std::hash::Hash + Eq + Clone + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    if size > 0 {
        Some(Cache::new(size))
    } else {
        None
    }
}

impl RpcCache {
    /// Creates a new [`RpcCache`] from the given configuration.
    pub fn new(config: &RpcCacheConfig) -> Self {
        Self {
            inner: Arc::new(RpcCacheInner {
                blocks_with_txs: make_cache(config.max_blocks_with_txs),
                blocks_with_tx_hashes: make_cache(config.max_blocks_with_tx_hashes),
                blocks_with_receipts: make_cache(config.max_blocks_with_receipts),
                transactions: make_cache(config.max_transactions),
                receipts: make_cache(config.max_receipts),
                classes: make_cache(config.max_classes),
                state_updates: make_cache(config.max_state_updates),
                traces: make_cache(config.max_traces),
                block_traces: make_cache(config.max_block_traces),
            }),
        }
    }

    // --- Blocks with transactions ---

    pub fn get_block_with_txs(&self, block_num: BlockNumber) -> Option<BlockWithTxs> {
        self.inner.blocks_with_txs.as_ref()?.get(&block_num)
    }

    pub fn insert_block_with_txs(&self, block_num: BlockNumber, block: BlockWithTxs) {
        if let Some(cache) = &self.inner.blocks_with_txs {
            cache.insert(block_num, block);
        }
    }

    // --- Blocks with transaction hashes ---

    pub fn get_block_with_tx_hashes(&self, block_num: BlockNumber) -> Option<BlockWithTxHashes> {
        self.inner.blocks_with_tx_hashes.as_ref()?.get(&block_num)
    }

    pub fn insert_block_with_tx_hashes(&self, block_num: BlockNumber, block: BlockWithTxHashes) {
        if let Some(cache) = &self.inner.blocks_with_tx_hashes {
            cache.insert(block_num, block);
        }
    }

    // --- Blocks with receipts ---

    pub fn get_block_with_receipts(&self, block_num: BlockNumber) -> Option<BlockWithReceipts> {
        self.inner.blocks_with_receipts.as_ref()?.get(&block_num)
    }

    pub fn insert_block_with_receipts(&self, block_num: BlockNumber, block: BlockWithReceipts) {
        if let Some(cache) = &self.inner.blocks_with_receipts {
            cache.insert(block_num, block);
        }
    }

    // --- Transactions ---

    pub fn get_transaction(&self, hash: TxHash) -> Option<RpcTxWithHash> {
        self.inner.transactions.as_ref()?.get(&hash)
    }

    pub fn insert_transaction(&self, hash: TxHash, tx: RpcTxWithHash) {
        if let Some(cache) = &self.inner.transactions {
            cache.insert(hash, tx);
        }
    }

    // --- Receipts ---

    pub fn get_receipt(&self, hash: TxHash) -> Option<TxReceiptWithBlockInfo> {
        self.inner.receipts.as_ref()?.get(&hash)
    }

    pub fn insert_receipt(&self, hash: TxHash, receipt: TxReceiptWithBlockInfo) {
        if let Some(cache) = &self.inner.receipts {
            cache.insert(hash, receipt);
        }
    }

    // --- Classes ---

    pub fn get_class(&self, key: (ClassHash, BlockNumber)) -> Option<Class> {
        self.inner.classes.as_ref()?.get(&key)
    }

    pub fn insert_class(&self, key: (ClassHash, BlockNumber), class: Class) {
        if let Some(cache) = &self.inner.classes {
            cache.insert(key, class);
        }
    }

    // --- State updates ---

    pub fn get_state_update(&self, block_num: BlockNumber) -> Option<ConfirmedStateUpdate> {
        self.inner.state_updates.as_ref()?.get(&block_num)
    }

    pub fn insert_state_update(&self, block_num: BlockNumber, update: ConfirmedStateUpdate) {
        if let Some(cache) = &self.inner.state_updates {
            cache.insert(block_num, update);
        }
    }

    // --- Transaction traces ---

    pub fn get_trace(&self, hash: TxHash) -> Option<TxTrace> {
        self.inner.traces.as_ref()?.get(&hash)
    }

    pub fn insert_trace(&self, hash: TxHash, trace: TxTrace) {
        if let Some(cache) = &self.inner.traces {
            cache.insert(hash, trace);
        }
    }

    // --- Block traces ---

    pub fn get_block_traces(&self, block_num: BlockNumber) -> Option<Vec<TxTraceWithHash>> {
        self.inner.block_traces.as_ref()?.get(&block_num)
    }

    pub fn insert_block_traces(&self, block_num: BlockNumber, traces: Vec<TxTraceWithHash>) {
        if let Some(cache) = &self.inner.block_traces {
            cache.insert(block_num, traces);
        }
    }
}
