use katana_primitives::transaction::TxHash;

use crate::ProviderResult;

/// Checkpoint for the messaging service.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MessagingCheckpoint {
    /// The settlement chain block number last successfully processed.
    pub block: u64,
    /// The transaction index within `block` up to which messages have been processed.
    pub tx_index: u64,
}

/// Provider for reading and writing messaging service checkpoints.
#[auto_impl::auto_impl(&, Box, Arc)]
pub trait MessagingCheckpointProvider: Send + Sync {
    /// Returns the last successfully processed checkpoint for the given messenger.
    fn messaging_checkpoint(&self, id: &str) -> ProviderResult<Option<MessagingCheckpoint>>;

    /// Sets the messaging checkpoint for the given messenger.
    fn set_messaging_checkpoint(
        &self,
        id: &str,
        checkpoint: &MessagingCheckpoint,
    ) -> ProviderResult<()>;
}

/// Read-only access to the settlement-chain L1 -> L2 index.
///
/// One L1 transaction may emit multiple `LogMessageToL2` (or `MessageSent`) events,
/// so the relationship is one-to-many.
#[auto_impl::auto_impl(&, Box, Arc)]
pub trait MessagingL1ToL2IndexProvider: Send + Sync {
    /// Returns all L2 L1Handler tx hashes spawned from the given settlement chain
    /// transaction. Returns an empty `Vec` if the L1 transaction is unknown to
    /// the index (either never seen or had no `MessageSent` events).
    fn l2_txs_for_l1(&self, l1_tx_hash: &[u8; 32]) -> ProviderResult<Vec<TxHash>>;
}

/// Write access to the settlement-chain L1 -> L2 index.
#[auto_impl::auto_impl(&, Box, Arc)]
pub trait MessagingL1ToL2IndexWriter: Send + Sync {
    /// Append an L2 tx hash to the index for the given L1 transaction.
    /// Idempotent: re-recording the same `(l1, l2)` pair is a no-op.
    fn record_l1_to_l2(&self, l1_tx_hash: &[u8; 32], l2_tx_hash: TxHash) -> ProviderResult<()>;
}
