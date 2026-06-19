use katana_primitives::block::BlockNumber;

use crate::ProviderResult;

/// Read access to the embedded settlement service's progress.
#[auto_impl::auto_impl(&, Box, Arc)]
pub trait SettlementCheckpointProvider: Send + Sync {
    /// Returns the most recent block settled to the settlement chain, or `None` if nothing has been
    /// settled yet (or this node never ran the settlement service).
    fn settled_block(&self) -> ProviderResult<Option<BlockNumber>>;
}

/// Write access to the embedded settlement service's progress.
#[auto_impl::auto_impl(&, Box, Arc)]
pub trait SettlementCheckpointWriter: Send + Sync {
    /// Records the most recent block settled to the settlement chain.
    fn set_settled_block(&self, block: BlockNumber) -> ProviderResult<()>;
}
