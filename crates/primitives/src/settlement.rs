use alloy_primitives::Bytes;
use serde::{Deserialize, Serialize};

/// An opaque identifier of a settlement proof, whose meaning is defined by the proving backend that
/// produced it.
///
/// The producing prover is a node-level constant (the settlement backend from the node config), so
/// it is not stored alongside the id — it can be inferred from config. For the SP1 backend this is
/// the 32-byte Succinct prover-network request ID, the value the Succinct explorer indexes at
/// `/request/<id>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(::arbitrary::Arbitrary))]
#[serde(transparent)]
pub struct ProofId(pub Bytes);

impl ProofId {
    /// Builds a proof id from its opaque bytes.
    pub fn new(id: impl Into<Bytes>) -> Self {
        Self(id.into())
    }
}

/// A reference to a generated-but-not-yet-settled batch proof.
///
/// Recorded by the settlement service the moment proving completes — before the `update_state`
/// submission — so that a node restart can recover the proof from the proving network by its
/// [`ProofId`] instead of paying for a fresh proving round. Cleared once the batch settles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(::arbitrary::Arbitrary))]
pub struct PendingBatchProof {
    /// First block of the unsettled batch (inclusive).
    pub first: u64,
    /// Last block of the unsettled batch (inclusive).
    pub last: u64,
    /// Reference to the generated proof, resolvable against the proving backend's network.
    pub proof: ProofId,
}
