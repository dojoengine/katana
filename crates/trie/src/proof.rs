use bitvec::order::Msb0;
use bitvec::prelude::BitVec;
use katana_primitives::Felt;
use starknet_types_core::hash::StarkHash;

use crate::node::{BinaryNode, EdgeNode};

/// A proof node in a Merkle proof.
#[derive(Debug, Clone, PartialEq)]
pub enum ProofNode {
    Binary { left: Felt, right: Felt },
    Edge { child: Felt, path: BitVec<u8, Msb0> },
}

impl ProofNode {
    /// Computes the hash of this proof node.
    pub fn hash<H: StarkHash>(&self) -> Felt {
        match self {
            ProofNode::Binary { left, right } => BinaryNode::calculate_hash::<H>(*left, *right),
            ProofNode::Edge { child, path } => EdgeNode::calculate_hash::<H>(*child, path),
        }
    }
}

/// Verifies a multiproof and returns the leaf values for each key.
pub fn verify_proof<H: StarkHash>(
    proofs: &[Vec<(ProofNode, Felt)>],
    root: Felt,
    keys: &[BitVec<u8, Msb0>],
) -> anyhow::Result<Vec<Felt>> {
    let mut results = Vec::with_capacity(keys.len());

    for (proof, key) in proofs.iter().zip(keys.iter()) {
        // Verify the proof chain: each node's computed hash should match the node_hash
        if let Some((first_node, first_hash)) = proof.first() {
            // The first node's hash should match the root
            let computed = first_node.hash::<H>();
            anyhow::ensure!(
                computed == *first_hash && *first_hash == root,
                "Proof root hash mismatch: computed {computed:#x}, expected {root:#x}"
            );
        }

        // Walk the proof to find the leaf value
        let mut current_hash = root;
        let mut height = 0;

        for (node, node_hash) in proof {
            anyhow::ensure!(
                *node_hash == current_hash,
                "Proof hash chain broken at height {height}"
            );

            match node {
                ProofNode::Binary { left, right } => {
                    if key[height] {
                        current_hash = *right;
                    } else {
                        current_hash = *left;
                    }
                    height += 1;
                }
                ProofNode::Edge { child, path } => {
                    current_hash = *child;
                    height += path.len();
                }
            }
        }

        results.push(current_hash);
    }

    Ok(results)
}
