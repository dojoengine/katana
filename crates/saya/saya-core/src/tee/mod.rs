//! TEE (Trusted Execution Environment) types and pipeline stages.
//!
//! Pipeline flow:
//!   `Vec<BlockInfo>` → [`TeeAttestor`] → [`TeeAttestation`]
//!                    → (TEE Prover — see [`crate::prover::tee`])
//!                    → [`crate::prover::tee::TeeProof`]

use katana_rpc_types::tee::BlockAttestation;

use crate::block_ingestor::BlockInfo;
use crate::prover::HasBlockNumber;

/// Attestation data fetched from a Katana rollup node for a batch of blocks.
///
/// The `blocks` are threaded through the entire TEE pipeline so that downstream stages
/// (particularly the settlement adapter) retain access to the original [`BlockInfo`] range
/// without needing an extra DB round-trip.
#[derive(Debug, Clone)]
pub struct TeeAttestation {
    /// The ordered batch of blocks covered by this attestation.
    pub blocks: Vec<BlockInfo>,
    pub attestation: BlockAttestation,
}

impl HasBlockNumber for TeeAttestation {
    /// Returns the block number of the last block in the batch — used for pipeline ordering.
    fn block_number(&self) -> u64 {
        self.blocks.last().expect("non-empty attestation batch").number
    }
}
