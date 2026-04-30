mod persistent_tee;
pub use persistent_tee::{PersistentTeeOrchestrator, PersistentTeeOrchestratorBuilder};

mod tee;
pub use tee::{TeeOrchestrator, TeeOrchestratorBuilder};

#[derive(Debug)]
pub struct Genesis {
    /// Number or height of the first block that transforms the genesis state. This is usually `0`
    /// or `1` for a new network, but can be arbitrary for existing networks.
    pub first_block_number: u64,
}
