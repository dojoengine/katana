//! Shared, observable progress of the embedded settlement service.
//!
//! The settle loop is the sole writer; the `katana_settlementStatus` RPC is the reader. A
//! [`SettlementStatusHandle`] is created at node build time and a clone handed to the RPC handler
//! even when settlement is disabled — in which case the settled block stays at `0`. (The chain
//! head reported alongside it is read live from the provider by the RPC handler, not from here.)

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use katana_primitives::block::BlockNumber;

/// Handle to the embedded settlement service's most recent settled block. Cheap to clone
/// (`Arc` inside).
#[derive(Debug, Clone, Default)]
pub struct SettlementStatusHandle {
    settled_block: Arc<AtomicU64>,
}

impl SettlementStatusHandle {
    /// A handle for a node that does not run the settlement service. Reports `0`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the most recent settled block (the Piltover on-chain cursor).
    pub fn set_settled(&self, block: BlockNumber) {
        self.settled_block.store(block, Ordering::Relaxed);
    }

    /// The most recent settled block, or `0` if nothing has been settled.
    pub fn settled_block(&self) -> BlockNumber {
        self.settled_block.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_by_default() {
        assert_eq!(SettlementStatusHandle::new().settled_block(), 0);
    }

    #[test]
    fn reflects_settled_writes() {
        let handle = SettlementStatusHandle::new();
        handle.set_settled(5);
        assert_eq!(handle.settled_block(), 5);
    }
}
