use core::integer::u512;
use core::poseidon::hades_permutation;
use katana_tee::katana_report_utils::verify_katana_report_data;

const POW_32_U128: u128 = 0x100000000;
const POW_64_U128: u128 = 0x10000000000000000;
const POW_96_U128: u128 = 0x1000000000000000000000000;

#[test]
fn test_verify_katana_report_data_layout() {
    let state_root: felt252 = 1;
    let block_hash: felt252 = 2;
    let (commitment, _, _) = hades_permutation(state_root, block_hash, 2);
    let commitment_u256: u256 = commitment.into();

    let report_data = u512 {
        limb0: swap_endian_u128(commitment_u256.high),
        limb1: swap_endian_u128(commitment_u256.low),
        limb2: 0,
        limb3: 0,
    };

    assert(
        verify_katana_report_data(report_data, state_root, block_hash),
        'Verification should succeed',
    );
}

fn swap_endian_u128(value: u128) -> u128 {
    let w0: u128 = value % POW_32_U128;
    let w1: u128 = (value / POW_32_U128) % POW_32_U128;
    let w2: u128 = (value / POW_64_U128) % POW_32_U128;
    let w3: u128 = value / POW_96_U128;

    let sw0: u128 = swap_endian_u32(w0.try_into().unwrap()).into();
    let sw1: u128 = swap_endian_u32(w1.try_into().unwrap()).into();
    let sw2: u128 = swap_endian_u32(w2.try_into().unwrap()).into();
    let sw3: u128 = swap_endian_u32(w3.try_into().unwrap()).into();

    (sw0 * POW_96_U128) + (sw1 * POW_64_U128) + (sw2 * POW_32_U128) + sw3
}

fn swap_endian_u32(word: u32) -> u32 {
    let b0 = word / 16777216;
    let b1 = (word / 65536) % 256;
    let b2 = (word / 256) % 256;
    let b3 = word % 256;
    (b3 * 16777216) + (b2 * 65536) + (b1 * 256) + b0
}
