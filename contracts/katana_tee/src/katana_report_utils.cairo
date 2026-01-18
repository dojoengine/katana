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
use core::integer::u512;

const POW_32_U128: u128 = 0x100000000;
const POW_64_U128: u128 = 0x10000000000000000;
const POW_96_U128: u128 = 0x1000000000000000000000000;


pub fn verify_katana_report_data(
    report_data: u512, state_root: felt252, block_hash: felt252,
) -> bool {
    assert(report_data.limb2 == 0, 'Report data limb2 must be 0');
    assert(report_data.limb3 == 0, 'Report data limb3 must be 0');

    let expected_commitment = u256 {
        low: swap_endian_u128(report_data.limb1), high: swap_endian_u128(report_data.limb0),
    };

    let (commitment, _, _) = hades_permutation(state_root, block_hash, 2);

    assert(commitment.into() == expected_commitment, 'Commitment mismatch');

    return true;
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
