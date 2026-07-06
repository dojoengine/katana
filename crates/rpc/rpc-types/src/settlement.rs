use katana_primitives::block::BlockNumber;
use katana_primitives::settlement::ProofId;
use serde::{Deserialize, Serialize};

/// Status of the node's embedded settlement service, returned by `katana_settlementStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettlementStatus {
    /// The current local chain head.
    pub head: BlockNumber,
    /// The most recent block settled to the settlement chain.
    pub settled_block: BlockNumber,
}

/// The proof that settled a given block, returned by `katana_getBlockProof`.
///
/// The proving backend is a node-level constant (inferable from config), so only the opaque proof
/// id is returned. For the `sp1` backend `proofId` is the `0x`-hex Succinct prover-network request
/// ID, the value the Succinct explorer indexes at `/request/<id>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockProof {
    /// The block this proof settled.
    pub block: BlockNumber,
    /// The opaque identifier of the proof, rendered as `0x`-hex.
    pub proof_id: ProofId,
}
