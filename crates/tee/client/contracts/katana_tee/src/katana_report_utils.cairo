use core::integer::{u128_byte_reverse, u512};
use core::poseidon::poseidon_hash_span;


/// Verify that report_data matches the Poseidon commitment.
///
/// commitment = poseidon_hash_span([state_root, block_hash, fork_block_number, events_commitment])
///
/// The TEE hardware embeds this hash in the attestation report's report_data field.
/// SP1 proves the report is authentic; this function verifies the hash binding.
pub fn verify_katana_report_data(
    report_data: u512,
    state_root: felt252,
    block_hash: felt252,
    fork_block_number: u64,
    events_commitment: felt252,
) -> bool {
    assert(report_data.limb2 == 0, 'Report data limb2 must be 0');
    assert(report_data.limb3 == 0, 'Report data limb3 must be 0');

    let expected_commitment = u256 {
        low: u128_byte_reverse(report_data.limb1), high: u128_byte_reverse(report_data.limb0),
    };

    let commitment = poseidon_hash_span(
        array![state_root, block_hash, fork_block_number.into(), events_commitment].span(),
    );

    assert(commitment.into() == expected_commitment, 'Commitment mismatch');

    return true;
}
