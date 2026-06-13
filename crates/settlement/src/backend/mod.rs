//! Proving backends.
//!
//! A [`ProvingBackend`] turns an unsettled block range into the
//! `update_state` payload for the settlement chain's core contract. The
//! settlement service is generic over this trait: the serial settle loop, the
//! on-chain cursor handling, and the transaction submission are
//! proof-system-agnostic, while everything specific to *how* a state
//! transition is proven lives behind the backend.
//!
//! [`tee::TeeBackend`] (AMD SEV-SNP attestations proven with SP1 Groth16) is
//! the only backend today; a standard validity-proof backend (e.g. SNOS +
//! STARK via the `LayoutBridgeOutput*` Piltover variants) would be a second
//! implementation of the same trait.

pub mod tee;

use async_trait::async_trait;
use katana_primitives::block::BlockNumber;

/// A proving system that can produce state-update payloads.
///
/// Backends own whatever they need to do their job (a provider handle for
/// chain reads, prover keys, hardware handles); the settlement service only
/// hands them the block range to settle.
///
/// [`Output`](Self::Output) is the payload type of the settlement chain's
/// core contract — a backend can only be paired (via
/// [`SettlementPipeline`](crate::SettlementPipeline)) with a
/// [`SettlementChain`](crate::chain::SettlementChain) that accepts it.
#[async_trait]
pub trait ProvingBackend: Send + Sync {
    /// The state-update payload this backend produces.
    type Output: Send + 'static;

    /// Human-readable backend name, for logs.
    fn name(&self) -> &'static str;

    /// Builds the `update_state` payload settling the state transition
    /// `(prev_block, block]`.
    ///
    /// `prev_block = None` is the genesis case: nothing has been settled yet
    /// and the range starts at block 0.
    async fn prove(
        &self,
        prev_block: Option<BlockNumber>,
        block: BlockNumber,
    ) -> Result<Self::Output, anyhow::Error>;
}
