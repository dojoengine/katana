pub mod classes;
pub mod contracts;
pub mod mem_storage;
pub mod node;
pub mod proof;
pub mod storage;
pub mod storages;
pub mod tree;

pub use bitvec;
use bitvec::view::AsBits;
pub use classes::*;
pub use contracts::*;
use katana_primitives::class::ClassHash;
use katana_primitives::Felt;
pub use mem_storage::MemStorage;
pub use node::{Node, NodeRef, StoredNode, TrieNodeIndex, TrieUpdate};
pub use proof::ProofNode;
use starknet_types_core::hash::{Pedersen, StarkHash};
pub use storage::Storage;
pub use storages::StoragesTrie;
pub use tree::MerkleTree;

/// A multi-proof: list of (ProofNode, hash) pairs representing proof paths.
/// This is a flat representation used for serialization/deserialization.
#[derive(Debug, Clone)]
pub struct MultiProof(pub Vec<(Felt, ProofNode)>);

/// Verifies a multi-proof against a root and a set of Felt keys.
/// Returns the leaf values for each key.
pub fn verify_proof<H: StarkHash>(proofs: &MultiProof, root: Felt, keys: Vec<Felt>) -> Vec<Felt> {
    // Convert to per-key paths (single path containing all nodes for backward compatibility)
    let bit_keys: Vec<_> = keys
        .iter()
        .map(|k| k.to_bytes_be().as_bits::<bitvec::order::Msb0>()[5..].to_owned())
        .collect();

    // Convert flat proof to per-key format by creating one path for all keys
    let paths: Vec<Vec<(ProofNode, Felt)>> = bit_keys
        .iter()
        .map(|_| proofs.0.iter().map(|(hash, node)| (node.clone(), *hash)).collect())
        .collect();

    proof::verify_proof::<H>(&paths, root, &bit_keys).unwrap_or_default()
}

/// A classes multi-proof with verify support using Poseidon hash.
#[derive(Debug, Clone)]
pub struct ClassesMultiProof(pub MultiProof);

impl ClassesMultiProof {
    pub fn verify(&self, root: Felt, class_hashes: Vec<ClassHash>) -> Vec<Felt> {
        verify_proof::<katana_primitives::hash::Poseidon>(&self.0, root, class_hashes)
    }
}

impl From<MultiProof> for ClassesMultiProof {
    fn from(value: MultiProof) -> Self {
        Self(value)
    }
}

/// Computes a Merkle root from a list of values using an in-memory trie.
pub fn compute_merkle_root<H>(values: &[Felt]) -> anyhow::Result<Felt>
where
    H: StarkHash + Send + Sync,
{
    let storage = MemStorage::new();
    let mut tree = MerkleTree::<H, 64>::empty();

    for (id, value) in values.iter().enumerate() {
        let key = bitvec::vec::BitVec::<u8, bitvec::order::Msb0>::from_iter(id.to_be_bytes());
        tree.set(&storage, key, *value)?;
    }

    let update = tree.commit(&storage)?;
    Ok(update.root_commitment)
}

/// Computes a Pedersen hash of two felts. Convenience wrapper for external callers.
pub fn pedersen_hash(a: &Felt, b: &Felt) -> Felt {
    Pedersen::hash(a, b)
}

/// Computes the contract state hash: `H(H(H(class_hash, storage_root), nonce), 0)`
/// where H is the Pedersen hash.
pub fn compute_contract_state_hash(
    class_hash: &ClassHash,
    storage_root: &Felt,
    nonce: &Felt,
) -> Felt {
    const CONTRACT_STATE_HASH_VERSION: Felt = Felt::ZERO;
    let hash = Pedersen::hash(class_hash, storage_root);
    let hash = Pedersen::hash(&hash, nonce);
    Pedersen::hash(&hash, &CONTRACT_STATE_HASH_VERSION)
}

#[cfg(test)]
mod tests {
    use katana_primitives::contract::Nonce;
    use katana_primitives::felt;
    use starknet_types_core::hash;

    use super::*;

    // Taken from Pathfinder: https://github.com/eqlabs/pathfinder/blob/29f93d0d6ad8758fdcf5ae3a8bd2faad2a3bc92b/crates/merkle-tree/src/transaction.rs#L70-L88
    #[test]
    fn test_commitment_merkle_tree() {
        let hashes = vec![Felt::from(1), Felt::from(2), Felt::from(3), Felt::from(4)];

        // Produced by the cairo-lang Python implementation:
        // `hex(asyncio.run(calculate_patricia_root([1, 2, 3, 4], height=64, ffc=ffc))))`
        let expected_root_hash =
            felt!("0x1a0e579b6b444769e4626331230b5ae39bd880f47e703b73fa56bf77e52e461");
        let computed_root_hash = compute_merkle_root::<hash::Pedersen>(&hashes).unwrap();

        assert_eq!(expected_root_hash, computed_root_hash);
    }

    // Taken from Pathfinder: https://github.com/eqlabs/pathfinder/blob/29f93d0d6ad8758fdcf5ae3a8bd2faad2a3bc92b/crates/merkle-tree/src/contract_state.rs#L236C5-L252C6
    #[test]
    fn test_compute_contract_state_hash() {
        let root = felt!("0x4fb440e8ca9b74fc12a22ebffe0bc0658206337897226117b985434c239c028");
        let class_hash = felt!("0x2ff4903e17f87b298ded00c44bfeb22874c5f73be2ced8f1d9d9556fb509779");
        let nonce = Nonce::ZERO;

        let result = compute_contract_state_hash(&class_hash, &root, &nonce);
        let expected = felt!("0x7161b591c893836263a64f2a7e0d829c92f6956148a60ce5e99a3f55c7973f3");

        assert_eq!(result, expected);
    }

    #[test]
    fn test_set_and_commit() {
        use bitvec::prelude::*;

        let storage = MemStorage::new();
        let mut tree = MerkleTree::<hash::Pedersen, 251>::empty();

        // Insert some values
        let key1 = felt!("0x1").to_bytes_be().as_bits::<Msb0>()[5..].to_owned();
        let key2 = felt!("0x2").to_bytes_be().as_bits::<Msb0>()[5..].to_owned();

        tree.set(&storage, key1, Felt::from(100)).unwrap();
        tree.set(&storage, key2, Felt::from(200)).unwrap();

        let update = tree.commit(&storage).unwrap();
        assert_ne!(update.root_commitment, Felt::ZERO);
        assert!(!update.nodes_added.is_empty());
    }

    #[test]
    fn test_empty_tree_root_is_zero() {
        let storage = MemStorage::new();
        let tree = MerkleTree::<hash::Pedersen, 251>::empty();
        let update = tree.commit(&storage).unwrap();
        assert_eq!(update.root_commitment, Felt::ZERO);
    }

    #[test]
    fn test_multi_block_with_random_keys_mem_storage() {
        use bitvec::prelude::*;

        let mut storage = MemStorage::new();

        // Generate pseudo-random keys
        let mut rng_state = 0x12345678u64;
        let mut next_random_felt = || -> Felt {
            rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let hi = rng_state;
            rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let lo = rng_state;
            let mut bytes = [0u8; 32];
            bytes[16..24].copy_from_slice(&hi.to_be_bytes());
            bytes[24..32].copy_from_slice(&lo.to_be_bytes());
            bytes[0] &= 0x07;
            Felt::from_bytes_be(&bytes)
        };

        // Block 0: insert 10 keys
        let mut tree0 = MerkleTree::<hash::Pedersen, 251>::empty();
        for _ in 0..10 {
            let key_felt = next_random_felt();
            let value = next_random_felt();
            let key = key_felt.to_bytes_be().as_bits::<Msb0>()[5..].to_owned();
            tree0.set(&storage, key, value).unwrap();
        }

        let update0 = tree0.commit(&storage).unwrap();
        let root0_idx = storage.apply_update(&update0).unwrap();

        // Block 1: insert 10 more keys using block 0's root
        let mut tree1 = MerkleTree::<hash::Pedersen, 251>::new(root0_idx);
        for _ in 0..10 {
            let key_felt = next_random_felt();
            let value = next_random_felt();
            let key = key_felt.to_bytes_be().as_bits::<Msb0>()[5..].to_owned();
            tree1.set(&storage, key, value).unwrap();
        }

        let update1 = tree1.commit(&storage).unwrap();
        assert_ne!(update1.root_commitment, Felt::ZERO);
    }

    #[test]
    fn test_revert_by_root_switch() {
        use bitvec::prelude::*;

        let mut storage = MemStorage::new();

        // Block 0: insert values
        let mut tree0 = MerkleTree::<hash::Pedersen, 251>::empty();
        let key1 = felt!("0x1").to_bytes_be().as_bits::<Msb0>()[5..].to_owned();
        let key2 = felt!("0x2").to_bytes_be().as_bits::<Msb0>()[5..].to_owned();
        tree0.set(&storage, key1.clone(), Felt::from(100)).unwrap();
        tree0.set(&storage, key2.clone(), Felt::from(200)).unwrap();

        let update0 = tree0.commit(&storage).unwrap();
        let root0 = update0.root_commitment;
        let root0_idx = storage.apply_update(&update0).unwrap();

        // Block 1: insert more values
        let mut tree1 = MerkleTree::<hash::Pedersen, 251>::new(root0_idx);
        let key3 = felt!("0x3").to_bytes_be().as_bits::<Msb0>()[5..].to_owned();
        tree1.set(&storage, key3, Felt::from(300)).unwrap();

        let update1 = tree1.commit(&storage).unwrap();
        let root1 = update1.root_commitment;
        let _root1_idx = storage.apply_update(&update1);

        assert_ne!(root0, root1);

        // "Revert" to block 0 by using root0_idx
        let reverted_tree = MerkleTree::<hash::Pedersen, 251>::new(root0_idx);
        let reverted_update = reverted_tree.commit(&storage).unwrap();
        assert_eq!(reverted_update.root_commitment, root0);
    }
}
