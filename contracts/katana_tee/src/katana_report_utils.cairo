// /// The report data is: Poseidon(state_root, block_hash) padded to 64 bytes.
// fn compute_report_data(&self, state_root: Felt, block_hash: Felt) -> [u8; 64] {
//     // Compute Poseidon hash of state_root and block_hash
//     let commitment = Poseidon::hash(&state_root, &block_hash);

//     // Convert Felt to bytes (32 bytes) and pad to 64 bytes
//     let commitment_bytes = commitment.to_bytes_be();

//     let mut report_data = [0u8; 64];
//     // Place the 32-byte hash in the first half
//     report_data[..32].copy_from_slice(&commitment_bytes);
//     // Second half remains zeros (or could include additional metadata)

use core::poseidon::hades_permutation;
use core::integer::{u512, u128_byte_reverse};



pub fn verify_katana_report_data(
    report_data: u512, state_root: felt252, block_hash: felt252,
) -> bool {
    assert(report_data.limb2 == 0, 'Report data limb2 must be 0');
    assert(report_data.limb3 == 0, 'Report data limb3 must be 0');

    let expected_commitment = u256 {
        low: u128_byte_reverse(report_data.limb1), high: u128_byte_reverse(report_data.limb0),
    };

    let (commitment, _, _) = hades_permutation(state_root, block_hash, 2);

    assert(commitment.into() == expected_commitment, 'Commitment mismatch');

    return true;
}