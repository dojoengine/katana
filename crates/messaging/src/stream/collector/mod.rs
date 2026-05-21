//! Message collector trait and implementations.
//!
//! A [`MessageCollector`] knows how to fetch messages from a specific settlement chain.
//! It provides two operations:
//! - [`latest_block`](MessageCollector::latest_block) — get the latest block number on the chain
//! - [`gather`](MessageCollector::gather) — fetch positioned messages from a block range

use std::future::Future;
use std::pin::Pin;

use katana_primitives::block::BlockNumber;
use katana_primitives::chain::ChainId;
use katana_primitives::transaction::L1HandlerTx;

pub mod ethereum;
pub mod starknet;

/// A message gathered from the settlement chain together with the position
/// that identifies it uniquely.
///
/// The position `(block, tx_index)` is used for checkpointing. On restart, the
/// messaging server reads the persisted checkpoint and passes it back to the
/// collector to skip already-processed messages.
#[derive(Debug, Clone)]
pub struct OrderedMessage {
    /// The settlement block the message was emitted in.
    pub block: u64,

    /// The transaction index within `block`.
    ///
    /// For Ethereum, this is the L1 transaction index of the log. For Starknet,
    /// it is the position of the event among `MessageSent` events scoped to the
    /// block (Starknet events don't carry a native tx index).
    pub tx_index: u64,

    /// The settlement chain transaction hash that emitted the originating event/log.
    ///
    /// For Ethereum, this is the L1 transaction hash that called `sendMessageToL2`.
    /// For Starknet (L2 -> L3), this is the L2 transaction hash that emitted the
    /// `MessageSent` event. Both are 32-byte hashes; this is the raw bytes.
    pub l1_tx_hash: [u8; 32],

    /// The L1Handler transaction converted from the settlement chain event.
    pub tx: L1HandlerTx,
}

/// The result of a gather operation.
#[derive(Debug)]
pub struct GatherResult {
    /// The last settlement block inspected. `from_block` advances past this
    /// after a successful gather.
    pub to_block: u64,

    /// Messages gathered from the range `[from_block, to_block]`, already filtered
    /// to exclude any at or before the `from_tx_index` resume position in `from_block`.
    pub messages: Vec<OrderedMessage>,
}

/// A message collector fetches L1Handler messages from a settlement chain.
///
/// Implementations are chain-specific (Ethereum, Starknet) and handle the
/// details of log/event fetching and conversion to L1HandlerTx.
#[auto_impl::auto_impl(Arc)]
pub trait MessageCollector: Send + Sync + 'static {
    /// The error type returned by collector operations.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Get the latest block number on the settlement chain.
    fn latest_block(&self) -> Pin<Box<dyn Future<Output = Result<u64, Self::Error>> + Send + '_>>;

    /// Gather messages from the given block range.
    ///
    /// `from_tx_index` is the resume cursor within `from_block`: implementations must skip
    /// any message whose block equals `from_block` and whose tx index is `< from_tx_index`.
    /// For all other blocks in the range, every message is included.
    ///
    /// `from_tx_index` is `0` on a fresh start (no checkpoint) and on any gather where
    /// the previous run advanced `from_block` past all completed blocks.
    fn gather(
        &self,
        from_block: BlockNumber,
        from_tx_index: u64,
        to_block: BlockNumber,
        chain_id: ChainId,
    ) -> Pin<Box<dyn Future<Output = Result<GatherResult, Self::Error>> + Send + '_>>;
}
