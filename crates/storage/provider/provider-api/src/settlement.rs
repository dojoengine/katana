use katana_primitives::block::BlockNumber;
use katana_primitives::settlement::ProofId;

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

/// Read access to the block -> proof mapping recorded by the settlement service.
#[auto_impl::auto_impl(&, Box, Arc)]
pub trait SettlementProofProvider: Send + Sync {
    /// Returns a reference to the proof that settled `block`, or `None` if the block has not been
    /// settled (or was settled without a proof reference, e.g. mock mode).
    fn block_proof(&self, block: BlockNumber) -> ProviderResult<Option<ProofId>>;
}

/// Write access to the block -> proof mapping recorded by the settlement service.
#[auto_impl::auto_impl(&, Box, Arc)]
pub trait SettlementProofWriter: Send + Sync {
    /// Records the proof that settled `block`.
    fn set_block_proof(&self, block: BlockNumber, proof: ProofId) -> ProviderResult<()>;
}
