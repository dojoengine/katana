use bitvec::view::AsBits;
use katana_primitives::cairo::ShortString;
use katana_primitives::class::{ClassHash, CompiledClassHash};
use katana_primitives::hash::Poseidon;
use katana_primitives::Felt;
use starknet_types_core::hash::StarkHash;

use crate::node::{TrieNodeIndex, TrieUpdate};
use crate::proof::ProofNode;
use crate::storage::Storage;
use crate::tree::MerkleTree;

/// The classes trie — maps class hash to `Poseidon("CONTRACT_CLASS_LEAF_V0", compiled_hash)`.
pub struct ClassesTrie<S: Storage> {
    tree: MerkleTree<Poseidon, 251>,
    storage: S,
}

impl<S: Storage + std::fmt::Debug> std::fmt::Debug for ClassesTrie<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClassesTrie")
            .field("tree", &self.tree)
            .field("storage", &self.storage)
            .finish()
    }
}

impl<S: Storage> ClassesTrie<S> {
    pub fn new(storage: S, root: TrieNodeIndex) -> Self {
        Self { tree: MerkleTree::new(root), storage }
    }

    pub fn empty(storage: S) -> Self {
        Self { tree: MerkleTree::empty(), storage }
    }

    pub fn insert(
        &mut self,
        hash: ClassHash,
        compiled_hash: CompiledClassHash,
    ) -> anyhow::Result<()> {
        let value = compute_classes_trie_value(compiled_hash);
        let key = hash.to_bytes_be().as_bits::<bitvec::order::Msb0>()[5..].to_owned();
        self.tree.set(&self.storage, key, value)
    }

    pub fn commit(self) -> anyhow::Result<TrieUpdate> {
        self.tree.commit(&self.storage)
    }

    pub fn root_hash(&self) -> anyhow::Result<Felt> {
        // Commit a clone to compute the root without consuming self
        let update = self.tree.clone().commit(&self.storage)?;
        Ok(update.root_commitment)
    }

    pub fn proofs(
        &self,
        root: TrieNodeIndex,
        keys: &[ClassHash],
    ) -> anyhow::Result<Vec<Vec<(ProofNode, Felt)>>> {
        let bit_keys: Vec<_> = keys
            .iter()
            .map(|k| k.to_bytes_be().as_bits::<bitvec::order::Msb0>()[5..].to_owned())
            .collect();
        let refs: Vec<_> = bit_keys.iter().map(|k| k.as_bitslice()).collect();
        MerkleTree::<Poseidon, 251>::get_proofs(root, &self.storage, &refs)
    }
}

pub fn compute_classes_trie_value(compiled_class_hash: CompiledClassHash) -> Felt {
    // https://docs.starknet.io/architecture-and-concepts/network-architecture/starknet-state/#classes_trie
    const CONTRACT_CLASS_LEAF_V0: ShortString = ShortString::from_ascii("CONTRACT_CLASS_LEAF_V0");
    Poseidon::hash(&CONTRACT_CLASS_LEAF_V0.into(), &compiled_class_hash)
}
