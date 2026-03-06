use bitvec::view::AsBits;
use katana_primitives::hash::Pedersen;
use katana_primitives::{ContractAddress, Felt};

use crate::node::{TrieNodeIndex, TrieUpdate};
use crate::proof::ProofNode;
use crate::storage::Storage;
use crate::tree::MerkleTree;

/// The contracts trie — maps contract address to contract state hash.
pub struct ContractsTrie<S: Storage> {
    tree: MerkleTree<Pedersen, 251>,
    storage: S,
}

impl<S: Storage + std::fmt::Debug> std::fmt::Debug for ContractsTrie<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContractsTrie")
            .field("tree", &self.tree)
            .field("storage", &self.storage)
            .finish()
    }
}

impl<S: Storage> ContractsTrie<S> {
    pub fn new(storage: S, root: TrieNodeIndex) -> Self {
        Self { tree: MerkleTree::new(root), storage }
    }

    pub fn empty(storage: S) -> Self {
        Self { tree: MerkleTree::empty(), storage }
    }

    pub fn insert(&mut self, address: ContractAddress, state_hash: Felt) -> anyhow::Result<()> {
        let key =
            Felt::from(address).to_bytes_be().as_bits::<bitvec::order::Msb0>()[5..].to_owned();
        self.tree.set(&self.storage, key, state_hash)
    }

    pub fn commit(self) -> anyhow::Result<TrieUpdate> {
        self.tree.commit(&self.storage)
    }

    pub fn root_hash(&self) -> anyhow::Result<Felt> {
        let update = self.tree.clone().commit(&self.storage)?;
        Ok(update.root_commitment)
    }

    pub fn proofs(
        &self,
        root: TrieNodeIndex,
        addresses: &[ContractAddress],
    ) -> anyhow::Result<Vec<Vec<(ProofNode, Felt)>>> {
        let bit_keys: Vec<_> = addresses
            .iter()
            .map(|a| Felt::from(*a).to_bytes_be().as_bits::<bitvec::order::Msb0>()[5..].to_owned())
            .collect();
        let refs: Vec<_> = bit_keys.iter().map(|k| k.as_bitslice()).collect();
        MerkleTree::<Pedersen, 251>::get_proofs(root, &self.storage, &refs)
    }
}

/// Data needed to compute a contract's leaf hash.
#[derive(Debug, Default)]
pub struct ContractLeaf {
    pub class_hash: Option<Felt>,
    pub storage_root: Option<Felt>,
    pub nonce: Option<Felt>,
}
