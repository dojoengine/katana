//! Proving backends.
//!
//! A [`ProvingBackend`] turns an unsettled block range into the `update_state`
//! payload for the Piltover core contract. The settlement service is generic
//! over this trait: the serial settle loop, the on-chain cursor handling, and
//! the transaction submission are proof-system-agnostic, while everything
//! specific to *how* a state transition is proven lives behind the backend.
//!
//! [`tee::TeeBackend`] (AMD SEV-SNP attestations proven with SP1 Groth16) is
//! the only backend today; a standard validity-proof backend (e.g. SNOS +
//! STARK via the `LayoutBridgeOutput*` Piltover variants) would be a second
//! implementation of the same trait.

pub mod tee;

use async_trait::async_trait;
use katana_primitives::block::BlockNumber;
use katana_primitives::settlement::ProofId;
use piltover::PiltoverInput;

use crate::error::SettlementError;

/// A proving system that can produce Piltover `update_state` payloads.
///
/// Backends own whatever they need to do their job (a provider handle for
/// chain reads, prover keys, hardware handles); the settlement service only
/// hands them the block range to settle.
#[async_trait]
pub trait ProvingBackend: Send + Sync {
    /// Human-readable backend name, for logs.
    fn name(&self) -> &'static str;

    /// Short, label-friendly identifier of the proof system (e.g. `sp1`,
    /// `mock`), used as a metric label.
    fn proof_type(&self) -> &'static str;

    /// Builds the `update_state` payload settling the state transition
    /// `(prev_block, block]`, along with an optional [`ProofId`] to the
    /// generated proof (`None` for backends/modes without one, e.g. mock).
    ///
    /// `prev_block = None` is the genesis case: nothing has been settled yet
    /// and the range starts at block 0.
    async fn prove(
        &self,
        prev_block: Option<BlockNumber>,
        block: BlockNumber,
    ) -> Result<(PiltoverInput, Option<ProofId>), SettlementError>;

    /// Rebuilds the `update_state` payload for `(prev_block, block]` from an
    /// already-generated proof, referenced by the [`ProofId`] the matching
    /// [`Self::prove`] call returned — without a fresh (on the SP1 network,
    /// paid) proving round.
    ///
    /// Unlike [`Self::prove`], no proof reference is returned: the caller
    /// already holds `proof` — it is what recovery resolves.
    ///
    /// Returns `Ok(None)` when the backend cannot recover proofs (e.g. mock
    /// proving, which has no network to recover from — and no cost to save);
    /// the caller then falls back to [`Self::prove`]. `Err` means recovery was
    /// attempted and failed (e.g. the network no longer retains the proof) —
    /// also a fall-back-to-prove signal, but worth surfacing in logs.
    async fn recover(
        &self,
        prev_block: Option<BlockNumber>,
        block: BlockNumber,
        proof: &ProofId,
    ) -> Result<Option<PiltoverInput>, SettlementError> {
        let _ = (prev_block, block, proof);
        Ok(None)
    }
}
