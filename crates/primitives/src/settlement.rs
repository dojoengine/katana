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
