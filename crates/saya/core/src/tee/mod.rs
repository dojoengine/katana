//! TEE (Trusted Execution Environment) types and pipeline stages.
//!
//! Pipeline flow:
//!   `Vec<BlockInfo>` → [`TeeAttestor`] → [`TeeAttestation`]
//!                    → (TEE Prover — see [`crate::prover::tee`])
//!                    → [`crate::prover::tee::TeeProof`]

use starknet_types_core::felt::Felt;

use crate::{block_ingestor::BlockInfo, prover::HasBlockNumber};

/// L2→L1 message emitted by a contract execution.
#[derive(Debug, Clone)]
pub struct L2ToL1Message {
    pub from_address: Felt,
    pub to_address: Felt,
    pub payload: Vec<Felt>,
}

/// L1→L2 message derived from an L1Handler transaction.
#[derive(Debug, Clone)]
pub struct L1ToL2Message {
    pub from_address: Felt,
    pub to_address: Felt,
    pub selector: Felt,
    pub payload: Vec<Felt>,
    pub nonce: Felt,
}

/// Attestation data fetched from a Katana rollup node for a batch of blocks.
///
/// The `blocks` are threaded through the entire TEE pipeline so that downstream stages
/// (particularly the settlement adapter) retain access to the original [`BlockInfo`] range
/// without needing an extra DB round-trip.
#[derive(Debug, Clone)]
pub struct TeeAttestation {
    /// The ordered batch of blocks covered by this attestation.
    pub blocks: Vec<BlockInfo>,
    /// Raw attestation bytes returned by Katana (hex-encoded AMD SEV-SNP quote).
    pub quote: String,
    pub prev_state_root: String,
    pub state_root: String,
    pub prev_block_hash: String,
    pub block_hash: String,
    pub prev_block_number: Felt,
    pub block_number: Felt,
    /// Poseidon commitment over all L1↔L2 messages in the attested block range.
    pub messages_commitment: Felt,
    /// All L2→L1 messages emitted in the attested block range.
    pub l2_to_l1_messages: Vec<L2ToL1Message>,
    /// All L1→L2 messages processed in the attested block range.
    pub l1_to_l2_messages: Vec<L1ToL2Message>,
}

impl HasBlockNumber for TeeAttestation {
    /// Returns the block number of the last block in the batch — used for pipeline ordering.
    fn block_number(&self) -> u64 {
        self.blocks
            .last()
            .expect("non-empty attestation batch")
            .number
    }
}
