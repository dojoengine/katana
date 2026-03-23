//! Message collector trait and implementations.
//!
//! A [`MessageCollector`] knows how to fetch messages from a specific settlement chain.
//! It provides two operations:
//! - [`latest_block`](MessageCollector::latest_block) — get the latest block number on the chain
//! - [`gather`](MessageCollector::gather) — fetch messages from a block range

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use katana_primitives::chain::ChainId;
use katana_primitives::transaction::L1HandlerTx;

use crate::Error;

/// The result of a gather operation.
#[derive(Debug)]
pub struct GatherResult {
    /// The last block that was processed.
    pub to_block: u64,
    /// The transaction index within `to_block` of the last processed message.
    pub tx_index: u64,
    /// The L1Handler transactions gathered.
    pub transactions: Vec<L1HandlerTx>,
}

/// A message collector fetches L1Handler messages from a settlement chain.
///
/// Implementations are chain-specific (Ethereum, Starknet) and handle the
/// details of log/event fetching and conversion to L1HandlerTx.
pub trait MessageCollector: Send + Sync + 'static {
    /// Get the latest block number on the settlement chain.
    fn latest_block(&self) -> Pin<Box<dyn Future<Output = Result<u64, Error>> + Send + '_>>;

    /// Gather messages from the given block range.
    fn gather(
        &self,
        from_block: u64,
        to_block: u64,
        chain_id: ChainId,
    ) -> Pin<Box<dyn Future<Output = Result<GatherResult, Error>> + Send + '_>>;
}

/// Blanket impl so `Arc<C>` also implements `MessageCollector`.
impl<C: MessageCollector> MessageCollector for Arc<C> {
    fn latest_block(&self) -> Pin<Box<dyn Future<Output = Result<u64, Error>> + Send + '_>> {
        (**self).latest_block()
    }

    fn gather(
        &self,
        from_block: u64,
        to_block: u64,
        chain_id: ChainId,
    ) -> Pin<Box<dyn Future<Output = Result<GatherResult, Error>> + Send + '_>> {
        (**self).gather(from_block, to_block, chain_id)
    }
}
