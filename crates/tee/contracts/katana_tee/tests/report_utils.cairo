use core::integer::{u128_byte_reverse, u512};
use core::poseidon::poseidon_hash_span;
use katana_tee::katana_report_utils::verify_katana_report_data;

#[test]
fn test_verify_katana_report_data_layout() {
    let state_root: felt252 = 1;
    let block_hash: felt252 = 2;
    let fork_block_number: u64 = 0;
    let events_commitment: felt252 = 0;
    let commitment = poseidon_hash_span(
        array![state_root, block_hash, fork_block_number.into(), events_commitment].span(),
    );
    let commitment_u256: u256 = commitment.into();

    let report_data = u512 {
        limb0: u128_byte_reverse(commitment_u256.high),
        limb1: u128_byte_reverse(commitment_u256.low),
        limb2: 0,
        limb3: 0,
    };

    assert(
        verify_katana_report_data(
            report_data, state_root, block_hash, fork_block_number, events_commitment,
        ),
        'Verification should succeed',
    );
}

#[test]
fn test_verify_katana_report_data_with_fork_block() {
    let state_root: felt252 = 0x123;
    let block_hash: felt252 = 0x456;
    let fork_block_number: u64 = 42;
    let events_commitment: felt252 = 0x789;
    let commitment = poseidon_hash_span(
        array![state_root, block_hash, fork_block_number.into(), events_commitment].span(),
    );
    let commitment_u256: u256 = commitment.into();

    let report_data = u512 {
        limb0: u128_byte_reverse(commitment_u256.high),
        limb1: u128_byte_reverse(commitment_u256.low),
        limb2: 0,
        limb3: 0,
    };

    assert(
        verify_katana_report_data(
            report_data, state_root, block_hash, fork_block_number, events_commitment,
        ),
        'Verification should succeed',
    );
}
