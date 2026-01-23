use core::integer::{u512, u128_byte_reverse};
use core::poseidon::hades_permutation;
use katana_tee::katana_report_utils::verify_katana_report_data;

#[test]
fn test_verify_katana_report_data_layout() {
    let state_root: felt252 = 1;
    let block_hash: felt252 = 2;
    let (commitment, _, _) = hades_permutation(state_root, block_hash, 2);
    let commitment_u256: u256 = commitment.into();

    let report_data = u512 {
        limb0: u128_byte_reverse(commitment_u256.high),
        limb1: u128_byte_reverse(commitment_u256.low),
        limb2: 0,
        limb3: 0,
    };

    assert(
        verify_katana_report_data(report_data, state_root, block_hash),
        'Verification should succeed',
    );
}
