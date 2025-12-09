use katana_primitives::block::BlockNumber;

use crate::ProviderResult;

#[auto_impl::auto_impl(&, Box, Arc)]
pub trait StageCheckpointProvider: Send + Sync {
    /// Returns the number of the last block that was successfully processed by the stage.
    fn execution_checkpoint(&self, id: &str) -> ProviderResult<Option<BlockNumber>>;

    /// Sets the checkpoint for a stage to the given block number.
    fn set_execution_checkpoint(&self, id: &str, block_number: BlockNumber) -> ProviderResult<()>;

    /// Returns the number of the last block that was successfully pruned by the stage.
    fn prune_checkpoint(&self, id: &str) -> ProviderResult<Option<BlockNumber>>;

    /// Sets the prune checkpoint for a stage to the given block number.
    fn set_prune_checkpoint(&self, id: &str, block_number: BlockNumber) -> ProviderResult<()>;
}
