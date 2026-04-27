//! Storage proof verification and commitment computation for the ZK circuit.
//!
//! Full verification flow:
//! 1. Verify global_state_root = poseidon("STARKNET_STATE_V0", contracts_tree_root, classes_tree_root)
//! 2. Verify contract leaf is in contracts_tree at key=contract_address
//! 3. Verify storage keys/values are in contract's storage_root
//!
//! The commitment is poseidon_hash(storage_commitment, contract_address, nonce, global_state_root)
//! so the Cairo contract can verify with the same hash.
//!
//! The nonce is included to prevent replay attacks (Ethereum-style nonce pattern).
//! The contract_address binds the commitment to a specific contract.
//! The global_state_root binds the commitment to the attested state.
//!
//! Storage proofs are verified using [bonsai-trie](https://github.com/dojoengine/bonsai-trie)
//! (same format as Katana/Starknet): Pedersen hash, 251-bit keys (Felt), MultiProof from the trie.

use alloy_primitives::Bytes;
use anyhow::{ensure, Context};
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::{Pedersen, Poseidon, StarkHash};

/// Computes the storage commitment as poseidon_hash(keys || values || contract_address || nonce).
/// Matches Cairo: poseidon_hash_span([keys..., values..., contract_address, nonce]).
///
/// The nonce is included to prevent replay attacks (Ethereum-style nonce pattern).
/// The contract_address binds the commitment to a specific contract.
pub fn compute_storage_commitment(
    keys: &[Bytes],
    values: &[Bytes],
) -> Felt {
    let mut data = Vec::with_capacity(keys.len() + values.len() + 2);
    for key in keys {
        data.push(bytes_to_felt(key));
    }
    for value in values {
        data.push(bytes_to_felt(value));
    }
    Poseidon::hash_array(&data)
}

pub fn compute_commitment(
    storage_commitment: &Felt,
    contract_address: &[u8; 32],
    nonce: u64,
    global_state_root: &[u8; 32],
    end_block_number: u64,
) -> Felt {
    let mut data = Vec::with_capacity(5);
    data.push(*storage_commitment);
    data.push(Felt::from_bytes_be(contract_address));
    data.push(Felt::from(nonce));
    data.push(Felt::from_bytes_be(global_state_root));
    data.push(Felt::from(end_block_number));
    Poseidon::hash_array(&data)
}

/// Verifies an event inclusion proof against the events_commitment Merkle root.
///
/// Events use a BonsaiStorage trie with Poseidon hash and 64-bit keys (event index as u64).
/// The proof format is the same scale-encoded MultiProof as storage proofs.
pub fn verify_event_proof(
    events_commitment: &[u8; 32],
    event_hash: &[u8; 32],
    event_index: u32,
    events_count: u32,
    proof_nodes: &[Bytes],
) -> anyhow::Result<()> {
    ensure!(
        (event_index as usize) < events_count as usize,
        "event_index {} out of bounds (events_count {})",
        event_index,
        events_count
    );

    let root = Felt::from_bytes_be(events_commitment);
    let expected_value = Felt::from_bytes_be(event_hash);

    ensure!(expected_value != Felt::ZERO, "event hash must not be zero");

    let proof_bytes = proof_nodes
        .first()
        .context("event proof requires proof_nodes[0]")?;
    let multiproof = decode_bonsai_multiproof(proof_bytes)?;

    // Event trie key = event_index as u64, encoded as 8 big-endian bytes → 64 bits
    // NOTE: must use u64 (not usize) because SP1 is a 32-bit RISC-V VM where usize = 4 bytes
    let key_bytes = (event_index as u64).to_be_bytes();
    let key_bits = bitvec::vec::BitVec::<u8, bitvec::order::Msb0>::from_slice(&key_bytes);

    let verified_values: Vec<Felt> = multiproof
        .verify_proof::<Poseidon>(root, std::iter::once(key_bits.as_bitslice()), 64)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| anyhow::anyhow!("event proof verification failed: {}", e))?;

    ensure!(
        verified_values.len() == 1,
        "event proof should return exactly 1 value, got {}",
        verified_values.len()
    );
    ensure!(
        verified_values[0] == expected_value,
        "event proof value mismatch: expected {:#x}, got {:#x}",
        expected_value,
        verified_values[0]
    );

    Ok(())
}

/// STARKNET_STATE_V0 short string as Felt
const STARKNET_STATE_V0: Felt = Felt::from_hex_unchecked("0x535441524b4e45545f53544154455f5630");

/// Verifies that global_state_root = poseidon("STARKNET_STATE_V0", contracts_tree_root, classes_tree_root)
pub fn verify_global_state_root(
    global_state_root: &[u8; 32],
    contracts_tree_root: &[u8; 32],
    classes_tree_root: &[u8; 32],
) -> anyhow::Result<()> {
    let expected = Poseidon::hash_array(&[
        STARKNET_STATE_V0,
        Felt::from_bytes_be(contracts_tree_root),
        Felt::from_bytes_be(classes_tree_root),
    ]);
    let actual = Felt::from_bytes_be(global_state_root);
    ensure!(
        expected == actual,
        "global_state_root mismatch: expected {:#x}, got {:#x}",
        expected,
        actual
    );
    Ok(())
}

/// Computes the contract leaf hash for Starknet contracts trie.
/// contract_leaf_hash = H(H(H(class_hash, storage_root), nonce), 0)
/// where H = Pedersen hash
pub fn compute_contract_leaf_hash(
    class_hash: &[u8; 32],
    storage_root: &[u8; 32],
    nonce: u64,
) -> Felt {
    let class_hash_felt = Felt::from_bytes_be(class_hash);
    let storage_root_felt = Felt::from_bytes_be(storage_root);
    let nonce_felt = Felt::from(nonce);

    // H(H(H(class_hash, storage_root), nonce), 0)
    let h1 = Pedersen::hash(&class_hash_felt, &storage_root_felt);
    let h2 = Pedersen::hash(&h1, &nonce_felt);
    Pedersen::hash(&h2, &Felt::ZERO)
}

/// Verifies that a contract with expected storage_root exists in contracts_tree.
///
/// Steps:
/// 1. Compute expected leaf hash from (class_hash, storage_root, contract_nonce)
/// 2. Verify that contract_address maps to this leaf hash in contracts_tree
pub fn verify_contracts_proof(
    contracts_tree_root: &[u8; 32],
    contract_address: &[u8; 32],
    class_hash: &[u8; 32],
    storage_root: &[u8; 32],
    contract_nonce: u64,
    proof_nodes: &[Bytes],
) -> anyhow::Result<()> {
    if proof_nodes.is_empty() {
        anyhow::bail!("contracts proof requires at least one proof node");
    }

    let expected_leaf = compute_contract_leaf_hash(class_hash, storage_root, contract_nonce);
    let root = Felt::from_bytes_be(contracts_tree_root);

    // Decode and verify using bonsai-trie
    let proof_bytes = proof_nodes
        .first()
        .context("contracts proof requires proof_nodes[0]")?;
    let multiproof = decode_bonsai_multiproof(proof_bytes)?;

    let key_bits = felt_to_trie_key_bits_bytes(contract_address)?;
    let verified_values: Vec<Felt> = multiproof
        .verify_proof::<Pedersen>(root, std::iter::once(key_bits.as_bitslice()), 251)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| anyhow::anyhow!("contracts proof verification failed: {}", e))?;

    ensure!(
        verified_values.len() == 1,
        "contracts proof should return exactly 1 value, got {}",
        verified_values.len()
    );

    let actual_leaf = verified_values[0];
    ensure!(
        actual_leaf == expected_leaf,
        "contract leaf mismatch: expected {:#x}, got {:#x}. \
         This means the storage_root doesn't match what's in contracts_tree.",
        expected_leaf,
        actual_leaf
    );

    Ok(())
}

fn bytes_to_felt(b: &Bytes) -> Felt {
    let arr: [u8; 32] = b.as_ref().try_into().expect("expected 32 bytes");
    Felt::from_bytes_be(&arr)
}

fn felt_to_trie_key_bits_bytes(b: &[u8; 32]) -> anyhow::Result<bitvec::vec::BitVec<u8, bitvec::order::Msb0>> {
    use bitvec::prelude::*;
    let felt = Felt::from_bytes_be(b);
    let bytes = felt.to_bytes_be();
    let bits: BitVec<u8, Msb0> = BitVec::from_slice(&bytes);
    // Skip top 5 bits (256 - 251 = 5)
    Ok(bits[5..].to_bitvec())
}

/// Verifies a storage proof using bonsai-trie (Katana/Starknet format).
///
/// - `state_root`: 32 bytes (Felt)
/// - `keys` / `values`: each 32 bytes (Felt). Keys become trie paths: Felt → 251 bits (skip top 5 bits)
/// - `proof_nodes`: one element = scale-encoded MultiProof: `u32 len` then for each node:
///   `Felt key`, `u8 tag` (0=Binary, 1=Edge), then Binary: `Felt left`, `Felt right`;
///   Edge: `Felt child`, `Path path`. Same format as produced by Katana's trie.multiproof().
pub fn verify_storage_proof(
    state_root: &[u8; 32],
    keys: &[Bytes],
    values: &[Bytes],
    proof_nodes: &[Bytes],
) -> anyhow::Result<()> {
    ensure!(
        keys.len() == values.len(),
        "keys and values length mismatch: {} vs {}",
        keys.len(),
        values.len()
    );
    if keys.is_empty() {
        ensure!(
            proof_nodes.is_empty() && state_root == &[0u8; 32],
            "empty keys/values requires empty proof and zero root"
        );
        return Ok(());
    }

    verify_storage_proof_bonsai(state_root, keys, values, proof_nodes)
}

fn verify_storage_proof_bonsai(
    state_root: &[u8; 32],
    keys: &[Bytes],
    values: &[Bytes],
    proof_nodes: &[Bytes],
) -> anyhow::Result<()> {
    use starknet_types_core::felt::Felt;
    use starknet_types_core::hash::Pedersen;

    let proof_bytes = proof_nodes
        .first()
        .context("bonsai proof requires one scale-encoded MultiProof in proof_nodes[0]")?;
    let multiproof = decode_bonsai_multiproof(proof_bytes)?;

    let root = felt_from_bytes(state_root)?;
    let key_bits: Vec<_> = keys
        .iter()
        .map(|k| felt_to_trie_key_bits(k))
        .collect::<anyhow::Result<_>>()?;

    let verified_values: Vec<Felt> = multiproof
        .verify_proof::<Pedersen>(root, key_bits.iter().map(|b| b.as_bitslice()), 251)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| anyhow::anyhow!("bonsai proof verification failed: {}", e))?;

    ensure!(
        verified_values.len() == values.len(),
        "proof returned {} values, expected {}",
        verified_values.len(),
        values.len()
    );
    for (i, (got, expected_bytes)) in verified_values.iter().zip(values.iter()).enumerate() {
        let expected = felt_from_bytes(expected_bytes.as_ref())?;
        ensure!(
            got == &expected,
            "value mismatch at index {}: proof gave {:?}, expected {:?}",
            i,
            got,
            expected
        );
    }
    Ok(())
}

fn felt_from_bytes(b: &[u8]) -> anyhow::Result<starknet_types_core::felt::Felt> {
    let arr: [u8; 32] = b
        .try_into()
        .map_err(|_| anyhow::anyhow!("expected 32 bytes for Felt, got {}", b.len()))?;
    Ok(starknet_types_core::felt::Felt::from_bytes_be(&arr))
}

fn felt_to_trie_key_bits(
    key: &Bytes,
) -> anyhow::Result<bitvec::prelude::BitVec<u8, bitvec::order::Msb0>> {
    use bitvec::view::AsBits;
    let b = key.as_ref();
    ensure!(b.len() >= 32, "key must be at least 32 bytes for Felt");
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&b[b.len() - 32..]);
    let felt = starknet_types_core::felt::Felt::from_bytes_be(&arr);
    let bytes = felt.to_bytes_be();
    let bits = bytes.as_bits::<bitvec::order::Msb0>();
    let trie_bits = bits[5..].to_owned();
    Ok(trie_bits)
}

/// Decodes scale-encoded MultiProof (IndexMap<Felt, ProofNode>).
fn decode_bonsai_multiproof(bytes: &Bytes) -> anyhow::Result<bonsai_trie::MultiProof> {
    use bonsai_trie::{Path, ProofNode};
    use indexmap::IndexMap;
    use parity_scale_codec::Decode;
    use starknet_types_core::felt::Felt;

    let mut r = bytes.as_ref();
    let len = u32::decode(&mut r).context("decode proof len")? as usize;
    let mut map = IndexMap::with_capacity(len);
    for _ in 0..len {
        let key = Felt::decode(&mut r).context("decode proof node key")?;
        let tag = u8::decode(&mut r).context("decode proof node tag")?;
        let node = match tag {
            0 => {
                let left = Felt::decode(&mut r).context("decode Binary left")?;
                let right = Felt::decode(&mut r).context("decode Binary right")?;
                ProofNode::Binary { left, right }
            }
            1 => {
                let child = Felt::decode(&mut r).context("decode Edge child")?;
                let path = Path::decode(&mut r).context("decode Edge path")?;
                ProofNode::Edge { child, path }
            }
            _ => anyhow::bail!("invalid proof node tag {}", tag),
        };
        map.insert(key, node);
    }
    Ok(bonsai_trie::MultiProof(map))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::Bytes;
    use bitvec::view::AsBits;
    use bonsai_trie::{
        databases::HashMapDb, id::BasicIdBuilder, BonsaiStorage, BonsaiStorageConfig, MultiProof,
    };
    use parity_scale_codec::Encode;
    use starknet_types_core::felt::Felt;
    use starknet_types_core::hash::Pedersen;

    /// Helper for building test tries and generating proofs
    struct TestTrie {
        storage: BonsaiStorage<bonsai_trie::id::BasicId, HashMapDb<bonsai_trie::id::BasicId>, Pedersen>,
        entries: Vec<(Felt, Felt)>,
    }

    impl TestTrie {
        fn new() -> Self {
            let config = BonsaiStorageConfig::default();
            let db = HashMapDb::<bonsai_trie::id::BasicId>::default();
            let storage = BonsaiStorage::<_, _, Pedersen>::new(db, config, 251);
            Self { storage, entries: vec![] }
        }

        fn insert(&mut self, key: Felt, value: Felt) {
            let identifier = b"test";
            let key_bits = key.to_bytes_be().as_bits::<bitvec::order::Msb0>()[5..].to_owned();
            self.storage.insert(identifier, &key_bits, &value).unwrap();
            self.entries.push((key, value));
        }

        fn commit(&mut self) -> Felt {
            let identifier = b"test";
            let id = BasicIdBuilder::new().new_id();
            self.storage.commit(id).unwrap();
            self.storage.root_hash(identifier).unwrap()
        }

        fn get_proof(&mut self, query_keys: &[Felt]) -> MultiProof {
            let identifier = b"test";
            let keys_bits: Vec<_> = query_keys
                .iter()
                .map(|k| k.to_bytes_be().as_bits::<bitvec::order::Msb0>()[5..].to_owned())
                .collect();
            self.storage
                .get_multi_proof(identifier, keys_bits.iter().map(|b| b.as_bitslice()))
                .unwrap()
        }

        fn encode_proof(multiproof: &MultiProof) -> Vec<u8> {
            let mut encoded = Vec::new();
            (multiproof.0.len() as u32).encode_to(&mut encoded);
            for (node_key, node) in multiproof.0.iter() {
                node_key.encode_to(&mut encoded);
                match node {
                    bonsai_trie::ProofNode::Binary { left, right } => {
                        0u8.encode_to(&mut encoded);
                        left.encode_to(&mut encoded);
                        right.encode_to(&mut encoded);
                    }
                    bonsai_trie::ProofNode::Edge { child, path } => {
                        1u8.encode_to(&mut encoded);
                        child.encode_to(&mut encoded);
                        path.encode_to(&mut encoded);
                    }
                }
            }
            encoded
        }
    }

    #[test]
    fn test_compute_storage_commitment() {
        let key = Bytes::from(
            hex::decode("007ebcc807b5c7e19f245995a55aed6f46f5f582f476a886b91b834b0ddf5854")
                .unwrap(),
        );
        let value = Bytes::from(
            hex::decode("0000000000000000000000000000000000000000000000000000000000000003")
                .unwrap(),
        );

        let storage_commitment = compute_storage_commitment(&[key], &[value]);

        // storage_commitment = poseidon_hash([key, value])
        // This matches the Cairo test: poseidon_hash_span([key, value])
        assert_eq!(
            storage_commitment,
            Felt::from_hex("0x5c5e91c4a356d59920f05d3f642f2ac12d6bb5820d4e824fae01d2228712385")
                .unwrap()
        );
    }

    #[test]
    fn test_compute_commitment_full_flow() {
        let key = Bytes::from(
            hex::decode("007ebcc807b5c7e19f245995a55aed6f46f5f582f476a886b91b834b0ddf5854")
                .unwrap(),
        );
        let value = Bytes::from(
            hex::decode("0000000000000000000000000000000000000000000000000000000000000003")
                .unwrap(),
        );

        // Step 1: Compute storage commitment (keys || values)
        let storage_commitment = compute_storage_commitment(&[key], &[value]);

        // Step 2: Wrap with contract_address, nonce, storage_state_root
        let contract_address = [0u8; 32];
        let nonce = 0u64;
        let storage_state_root = [0u8; 32];

        let commitment = compute_commitment(&storage_commitment, &contract_address, nonce, &storage_state_root, 0);

        // This is the final commitment that gets registered on-chain
        // commitment = poseidon_hash([storage_commitment, contract_address, nonce, storage_state_root, end_block_number])
        assert!(!commitment.to_bytes_be().iter().all(|&b| b == 0), "commitment should not be zero");
    }

    #[test]
    fn test_compute_commitment_different_nonce() {
        let key = Bytes::from(
            hex::decode("007ebcc807b5c7e19f245995a55aed6f46f5f582f476a886b91b834b0ddf5854")
                .unwrap(),
        );
        let value = Bytes::from(
            hex::decode("0000000000000000000000000000000000000000000000000000000000000003")
                .unwrap(),
        );

        let storage_commitment = compute_storage_commitment(&[key], &[value]);
        let contract_address = [0u8; 32];
        let storage_state_root = [0u8; 32];

        let commitment_nonce_0 = compute_commitment(&storage_commitment, &contract_address, 0, &storage_state_root, 0);
        let commitment_nonce_1 = compute_commitment(&storage_commitment, &contract_address, 1, &storage_state_root, 0);

        // Different nonces should produce different commitments (replay protection)
        assert_ne!(commitment_nonce_0, commitment_nonce_1);
    }

    #[test]
    fn test_compute_commitment_different_contract_address() {
        let key = Bytes::from(
            hex::decode("007ebcc807b5c7e19f245995a55aed6f46f5f582f476a886b91b834b0ddf5854")
                .unwrap(),
        );
        let value = Bytes::from(
            hex::decode("0000000000000000000000000000000000000000000000000000000000000003")
                .unwrap(),
        );

        let storage_commitment = compute_storage_commitment(&[key], &[value]);
        let contract_address_a = [0u8; 32];
        let mut contract_address_b = [0u8; 32];
        contract_address_b[31] = 1; // Different contract
        let storage_state_root = [0u8; 32];

        let commitment_a = compute_commitment(&storage_commitment, &contract_address_a, 0, &storage_state_root, 0);
        let commitment_b = compute_commitment(&storage_commitment, &contract_address_b, 0, &storage_state_root, 0);

        // Different contract addresses should produce different commitments
        assert_ne!(commitment_a, commitment_b);
    }

    #[test]
    fn test_verify_storage_proof_empty() {
        let state_root = [0u8; 32];
        let keys: Vec<Bytes> = vec![];
        let values: Vec<Bytes> = vec![];
        let proof_nodes: Vec<Bytes> = vec![];

        let result = verify_storage_proof(&state_root, &keys, &values, &proof_nodes);
        assert!(result.is_ok());
    }

    #[test]
    fn test_verify_storage_proof_empty_with_non_zero_root_fails() {
        let state_root = [1u8; 32];
        let keys: Vec<Bytes> = vec![];
        let values: Vec<Bytes> = vec![];
        let proof_nodes: Vec<Bytes> = vec![];

        let result = verify_storage_proof(&state_root, &keys, &values, &proof_nodes);
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_storage_proof_key_value_mismatch() {
        let state_root = [0u8; 32];
        let keys = vec![Bytes::from(vec![0u8; 32])];
        let values: Vec<Bytes> = vec![];
        let proof_nodes: Vec<Bytes> = vec![];

        let result = verify_storage_proof(&state_root, &keys, &values, &proof_nodes);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("mismatch"));
    }

    #[test]
    fn test_verify_storage_proof_single_entry() {
        let mut trie = TestTrie::new();

        let key = Felt::from_hex("0x007ebcc807b5c7e19f245995a55aed6f46f5f582f476a886b91b834b0ddf5854").unwrap();
        let value = Felt::from_hex("0x03").unwrap();
        trie.insert(key, value);

        let root = trie.commit();
        let multiproof = trie.get_proof(&[key]);
        let encoded = TestTrie::encode_proof(&multiproof);

        let state_root: [u8; 32] = root.to_bytes_be();
        let keys = vec![Bytes::from(key.to_bytes_be().to_vec())];
        let values = vec![Bytes::from(value.to_bytes_be().to_vec())];
        let proof_nodes = vec![Bytes::from(encoded)];

        let result = verify_storage_proof(&state_root, &keys, &values, &proof_nodes);
        assert!(result.is_ok(), "verify_storage_proof failed: {:?}", result.err());
    }

    #[test]
    fn test_verify_storage_proof_multiple_entries() {
        let mut trie = TestTrie::new();

        // Insert multiple entries
        let entries = vec![
            (Felt::from(1u64), Felt::from(100u64)),
            (Felt::from(2u64), Felt::from(200u64)),
            (Felt::from(3u64), Felt::from(300u64)),
            (Felt::from(1000u64), Felt::from(999u64)),
            (Felt::from_hex("0xdeadbeef").unwrap(), Felt::from_hex("0xcafe").unwrap()),
        ];

        for (k, v) in &entries {
            trie.insert(*k, *v);
        }

        let root = trie.commit();

        // Query all entries
        let query_keys: Vec<Felt> = entries.iter().map(|(k, _)| *k).collect();
        let multiproof = trie.get_proof(&query_keys);
        let encoded = TestTrie::encode_proof(&multiproof);

        let state_root: [u8; 32] = root.to_bytes_be();
        let keys: Vec<Bytes> = entries.iter().map(|(k, _)| Bytes::from(k.to_bytes_be().to_vec())).collect();
        let values: Vec<Bytes> = entries.iter().map(|(_, v)| Bytes::from(v.to_bytes_be().to_vec())).collect();
        let proof_nodes = vec![Bytes::from(encoded)];

        let result = verify_storage_proof(&state_root, &keys, &values, &proof_nodes);
        assert!(result.is_ok(), "verify_storage_proof with multiple entries failed: {:?}", result.err());
    }

    #[test]
    fn test_verify_storage_proof_partial_query() {
        let mut trie = TestTrie::new();

        // Insert 5 entries but only query 2
        trie.insert(Felt::from(1u64), Felt::from(100u64));
        trie.insert(Felt::from(2u64), Felt::from(200u64));
        trie.insert(Felt::from(3u64), Felt::from(300u64));
        trie.insert(Felt::from(4u64), Felt::from(400u64));
        trie.insert(Felt::from(5u64), Felt::from(500u64));

        let root = trie.commit();

        // Only query keys 2 and 4
        let query_keys = vec![Felt::from(2u64), Felt::from(4u64)];
        let query_values = vec![Felt::from(200u64), Felt::from(400u64)];

        let multiproof = trie.get_proof(&query_keys);
        let encoded = TestTrie::encode_proof(&multiproof);

        let state_root: [u8; 32] = root.to_bytes_be();
        let keys: Vec<Bytes> = query_keys.iter().map(|k| Bytes::from(k.to_bytes_be().to_vec())).collect();
        let values: Vec<Bytes> = query_values.iter().map(|v| Bytes::from(v.to_bytes_be().to_vec())).collect();
        let proof_nodes = vec![Bytes::from(encoded)];

        let result = verify_storage_proof(&state_root, &keys, &values, &proof_nodes);
        assert!(result.is_ok(), "partial query verification failed: {:?}", result.err());
    }

    #[test]
    fn test_verify_storage_proof_wrong_root_fails() {
        let mut trie = TestTrie::new();

        let key = Felt::from(42u64);
        let value = Felt::from(123u64);
        trie.insert(key, value);

        let _correct_root = trie.commit();
        let multiproof = trie.get_proof(&[key]);
        let encoded = TestTrie::encode_proof(&multiproof);

        // Use wrong root (all 1s instead of correct root)
        let wrong_root = [1u8; 32];
        let keys = vec![Bytes::from(key.to_bytes_be().to_vec())];
        let values = vec![Bytes::from(value.to_bytes_be().to_vec())];
        let proof_nodes = vec![Bytes::from(encoded)];

        let result = verify_storage_proof(&wrong_root, &keys, &values, &proof_nodes);
        assert!(result.is_err(), "should fail with wrong root");
    }

    #[test]
    fn test_verify_storage_proof_wrong_value_fails() {
        let mut trie = TestTrie::new();

        let key = Felt::from(42u64);
        let value = Felt::from(123u64);
        trie.insert(key, value);

        let root = trie.commit();
        let multiproof = trie.get_proof(&[key]);
        let encoded = TestTrie::encode_proof(&multiproof);

        let state_root: [u8; 32] = root.to_bytes_be();
        let keys = vec![Bytes::from(key.to_bytes_be().to_vec())];
        // Wrong value - 999 instead of 123
        let wrong_value = Felt::from(999u64);
        let values = vec![Bytes::from(wrong_value.to_bytes_be().to_vec())];
        let proof_nodes = vec![Bytes::from(encoded)];

        let result = verify_storage_proof(&state_root, &keys, &values, &proof_nodes);
        assert!(result.is_err(), "should fail with wrong value");
    }

    // ── Event proof tests ──────────────────────────────────────────────

    /// Helper for building event tries (Poseidon hash, 64-bit keys).
    /// Matches the trie built by katana-trie's compute_merkle_root.
    struct TestEventTrie {
        storage: BonsaiStorage<bonsai_trie::id::BasicId, HashMapDb<bonsai_trie::id::BasicId>, Poseidon>,
        values: Vec<Felt>,
    }

    impl TestEventTrie {
        fn new() -> Self {
            let config = BonsaiStorageConfig::default();
            let db = HashMapDb::<bonsai_trie::id::BasicId>::default();
            let storage = BonsaiStorage::<_, _, Poseidon>::new(db, config, 64);
            Self { storage, values: vec![] }
        }

        fn insert(&mut self, value: Felt) {
            let identifier = b"1";
            let index = self.values.len();
            let key = bitvec::vec::BitVec::<u8, bitvec::order::Msb0>::from_slice(
                &index.to_be_bytes(),
            );
            self.storage.insert(identifier, key.as_bitslice(), &value).unwrap();
            self.values.push(value);
        }

        fn commit(&mut self) -> Felt {
            let identifier = b"1";
            let id = BasicIdBuilder::new().new_id();
            self.storage.commit(id).unwrap();
            self.storage.root_hash(identifier).unwrap()
        }

        fn get_proof(&mut self, index: usize) -> bonsai_trie::MultiProof {
            let identifier = b"1";
            let key = bitvec::vec::BitVec::<u8, bitvec::order::Msb0>::from_slice(
                &index.to_be_bytes(),
            );
            self.storage
                .get_multi_proof(identifier, vec![key])
                .unwrap()
        }
    }

    #[test]
    fn test_verify_event_proof_single_event() {
        let mut trie = TestEventTrie::new();
        let event_hash = Felt::from_hex("0xdeadbeef").unwrap();
        trie.insert(event_hash);

        let root = trie.commit();
        let proof = trie.get_proof(0);
        let encoded = TestTrie::encode_proof(&proof);

        let result = verify_event_proof(
            &root.to_bytes_be(),
            &event_hash.to_bytes_be(),
            0,
            1,
            &[Bytes::from(encoded)],
        );
        assert!(result.is_ok(), "event proof failed: {:?}", result.err());
    }

    #[test]
    fn test_verify_event_proof_multiple_events() {
        let mut trie = TestEventTrie::new();
        let events: Vec<Felt> = (0..5).map(|i| Felt::from(100u64 + i)).collect();
        for ev in &events {
            trie.insert(*ev);
        }

        let root = trie.commit();

        // Prove event at index 3
        let proof = trie.get_proof(3);
        let encoded = TestTrie::encode_proof(&proof);

        let result = verify_event_proof(
            &root.to_bytes_be(),
            &events[3].to_bytes_be(),
            3,
            5,
            &[Bytes::from(encoded)],
        );
        assert!(result.is_ok(), "event proof for index 3 failed: {:?}", result.err());
    }

    #[test]
    fn test_verify_event_proof_wrong_root_fails() {
        let mut trie = TestEventTrie::new();
        let event_hash = Felt::from(42u64);
        trie.insert(event_hash);

        let _correct_root = trie.commit();
        let proof = trie.get_proof(0);
        let encoded = TestTrie::encode_proof(&proof);

        let wrong_root = [1u8; 32];
        let result = verify_event_proof(
            &wrong_root,
            &event_hash.to_bytes_be(),
            0,
            1,
            &[Bytes::from(encoded)],
        );
        assert!(result.is_err(), "should fail with wrong root");
    }

    #[test]
    fn test_verify_event_proof_wrong_hash_fails() {
        let mut trie = TestEventTrie::new();
        let event_hash = Felt::from(42u64);
        trie.insert(event_hash);

        let root = trie.commit();
        let proof = trie.get_proof(0);
        let encoded = TestTrie::encode_proof(&proof);

        let wrong_hash = Felt::from(999u64);
        let result = verify_event_proof(
            &root.to_bytes_be(),
            &wrong_hash.to_bytes_be(),
            0,
            1,
            &[Bytes::from(encoded)],
        );
        assert!(result.is_err(), "should fail with wrong event hash");
    }

    #[test]
    fn test_verify_event_proof_index_out_of_bounds() {
        let result = verify_event_proof(
            &[0u8; 32],
            &[0u8; 32],
            5,  // index >= count
            3,
            &[Bytes::from(vec![0u8])],
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("out of bounds"));
    }
}
