use starknet_rust_core::crypto::HashFunction;
use starknet_rust_core::types::Felt;
use alloy_primitives::{Bytes, B256};
use amd_sev_snp_attestation_verifier::compute_storage_commitment;

#[test]
fn test_commitment_matches_cairo() {
    // Same inputs as the proof fixture:
    // key = 0x007ebcc807b5c7e19f245995a55aed6f46f5f582f476a886b91b834b0ddf5854
    // value = 0x3
    let key = Felt::from_hex("0x7ebcc807b5c7e19f245995a55aed6f46f5f582f476a886b91b834b0ddf5854").unwrap();
    let value = Felt::from_hex("0x3").unwrap();

    // This uses hash_many which should be equivalent to poseidon_hash_span
    let commitment = HashFunction::poseidon().hash_many(&[key, value]);

    println!("Rust hash_many commitment: {:#x}", commitment);

    // Now test with the actual compute_commitment function from verifier
    let key_bytes = Bytes::from(hex::decode("007ebcc807b5c7e19f245995a55aed6f46f5f582f476a886b91b834b0ddf5854").unwrap());
    let value_bytes = Bytes::from(hex::decode("0000000000000000000000000000000000000000000000000000000000000003").unwrap());

    let verifier_commitment = compute_storage_commitment(&[key_bytes], &[value_bytes]);
    println!("Verifier compute_commitment: {:#x}", verifier_commitment);

    // Convert to B256 (same as in sp1-verifier/src/main.rs)
    let commitment_b256 = B256::from_slice(&verifier_commitment.to_bytes_be());
    println!("As B256: {:?}", commitment_b256);

    // Should match
    assert_eq!(format!("{:#x}", commitment), format!("{:#x}", verifier_commitment),
        "hash_many and compute_commitment should produce same result");
}
