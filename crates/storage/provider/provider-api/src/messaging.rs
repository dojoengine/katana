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
