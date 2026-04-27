//! Helper for contracts that receive calldata (abi.encode(keys, values)) and verify
//! it matches the ZK journal's storage_commitment: keccak256(calldata) ==
//! journal.storage_commitment.
//! Matches Solidity StorageCommitmentChecker.checkCommitment.

use core::keccak::compute_keccak_byte_array;

/// Returns true if commitment is non-zero and keccak256(calldata_encoded) equals the commitment.
/// commitment: journal.storage_commitment (from VerifierJournal)
/// calldata_encoded: abi.encode(keys, values) - same encoding the operator sent to the ZK circuit
#[inline(always)]
pub fn check_commitment(commitment: u256, calldata_encoded: ByteArray) -> bool {
    if commitment.low == 0 && commitment.high == 0 {
        return false;
    }
    let hash = compute_keccak_byte_array(@calldata_encoded);
    hash.low == commitment.low && hash.high == commitment.high
}
