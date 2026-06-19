//! Unit tests for verify_katana_report_data function.

use core::integer::{u128_byte_reverse, u512};
use core::poseidon::poseidon_hash_span;
use katana_tee::katana_report_utils::verify_katana_report_data;

fn build_report_data(
    state_root: felt252, block_hash: felt252, fork_block_number: u64, events_commitment: felt252,
) -> u512 {
    let commitment = poseidon_hash_span(
        array![state_root, block_hash, fork_block_number.into(), events_commitment].span(),
    );
    let commitment_u256: u256 = commitment.into();
    u512 {
        limb0: u128_byte_reverse(commitment_u256.high),
        limb1: u128_byte_reverse(commitment_u256.low),
        limb2: 0,
        limb3: 0,
    }
}

/// Test case 1: Large values
#[test]
fn test_verify_katana_report_data_case_1() {
    let state_root: felt252 = 0x0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef;
    let block_hash: felt252 = 0x00fedcba9876543210fedcba9876543210fedcba9876543210fedcba98765432;
    let fork_block_number: u64 = 0;
    let events_commitment: felt252 = 0x0;

    let report_data = build_report_data(
        state_root, block_hash, fork_block_number, events_commitment,
    );
    let result = verify_katana_report_data(
        report_data, state_root, block_hash, fork_block_number, events_commitment,
    );
    assert(result, 'Verification should pass');
}

/// Test case 2: Small values
#[test]
fn test_verify_katana_report_data_case_2() {
    let state_root: felt252 = 0x1;
    let block_hash: felt252 = 0x2;
    let fork_block_number: u64 = 0;
    let events_commitment: felt252 = 0x3;

    let report_data = build_report_data(
        state_root, block_hash, fork_block_number, events_commitment,
    );
    let result = verify_katana_report_data(
        report_data, state_root, block_hash, fork_block_number, events_commitment,
    );
    assert(result, 'Verification should pass');
}

/// Test case 3: Realistic block data
#[test]
fn test_verify_katana_report_data_case_3() {
    let state_root: felt252 = 0x04b1a39276c1df7ca78febcb6850f3649e826a3f6618e6ab30b48dcc948de1ad;
    let block_hash: felt252 = 0x06c5e8a47fb34d21c08e4eb6a91fa7bce3f2d5a490c8b7e1d26f43098a5bc7e2;
    let fork_block_number: u64 = 0;
    let events_commitment: felt252 =
        0x01a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2d3e4f5a6b7c8d9e0f1;

    let report_data = build_report_data(
        state_root, block_hash, fork_block_number, events_commitment,
    );
    let result = verify_katana_report_data(
        report_data, state_root, block_hash, fork_block_number, events_commitment,
    );
    assert(result, 'Verification should pass');
}

/// Test case 4: With non-zero fork_block_number
#[test]
fn test_verify_katana_report_data_with_fork_block() {
    let state_root: felt252 = 0x04b1a39276c1df7ca78febcb6850f3649e826a3f6618e6ab30b48dcc948de1ad;
    let block_hash: felt252 = 0x06c5e8a47fb34d21c08e4eb6a91fa7bce3f2d5a490c8b7e1d26f43098a5bc7e2;
    let fork_block_number: u64 = 12345;
    let events_commitment: felt252 = 0xabc;

    let report_data = build_report_data(
        state_root, block_hash, fork_block_number, events_commitment,
    );
    let result = verify_katana_report_data(
        report_data, state_root, block_hash, fork_block_number, events_commitment,
    );
    assert(result, 'Verification should pass');
}

/// Test case 5: Commitment mismatch (should panic)
#[test]
#[should_panic(expected: 'Commitment mismatch')]
fn test_verify_katana_report_data_mismatch() {
    let state_root: felt252 = 0x1;
    let block_hash: felt252 = 0x2;
    let fork_block_number: u64 = 0;
    let events_commitment: felt252 = 0x0;

    let report_data = build_report_data(
        state_root, block_hash, fork_block_number, events_commitment,
    );
    // Pass wrong state_root (0x3 instead of 0x1)
    verify_katana_report_data(report_data, 0x3, block_hash, fork_block_number, events_commitment);
}

/// Test case 6: Fork block mismatch (should panic)
#[test]
#[should_panic(expected: 'Commitment mismatch')]
fn test_verify_katana_report_data_fork_block_mismatch() {
    let state_root: felt252 = 0x1;
    let block_hash: felt252 = 0x2;
    let events_commitment: felt252 = 0x0;

    let report_data = build_report_data(state_root, block_hash, 100, events_commitment);
    // Pass wrong fork_block (200 instead of 100)
    verify_katana_report_data(report_data, state_root, block_hash, 200, events_commitment);
}

/// Test case 7: Events commitment mismatch (should panic)
#[test]
#[should_panic(expected: 'Commitment mismatch')]
fn test_verify_katana_report_data_events_commitment_mismatch() {
    let state_root: felt252 = 0x1;
    let block_hash: felt252 = 0x2;
    let fork_block_number: u64 = 0;

    let report_data = build_report_data(state_root, block_hash, fork_block_number, 0xaaa);
    // Pass wrong events_commitment (0xbbb instead of 0xaaa)
    verify_katana_report_data(report_data, state_root, block_hash, fork_block_number, 0xbbb);
}

/// Test that limb2 must be zero
#[test]
#[should_panic(expected: 'Report data limb2 must be 0')]
fn test_verify_katana_report_data_limb2_nonzero() {
    let report_data = u512 { limb0: 0, limb1: 0, limb2: 1, limb3: 0 };

    verify_katana_report_data(report_data, 0x123, 0x456, 0, 0x0);
}

/// Test that limb3 must be zero
#[test]
#[should_panic(expected: 'Report data limb3 must be 0')]
fn test_verify_katana_report_data_limb3_nonzero() {
    let report_data = u512 { limb0: 0, limb1: 0, limb2: 0, limb3: 1 };

    verify_katana_report_data(report_data, 0x123, 0x456, 0, 0x0);
}
