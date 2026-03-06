use bitvec::view::AsBits;
use katana_primitives::contract::{StorageKey, StorageValue};
use katana_primitives::hash::Pedersen;
use katana_primitives::{ContractAddress, Felt};

use crate::node::{TrieNodeIndex, TrieUpdate};
use crate::proof::ProofNode;
use crate::storage::Storage;
use crate::tree::MerkleTree;

/// Per-contract storage trie — maps storage keys to storage values.
pub struct StoragesTrie<S: Storage> {
    address: ContractAddress,
    tree: MerkleTree<Pedersen, 251>,
    storage: S,
}

impl<S: Storage + std::fmt::Debug> std::fmt::Debug for StoragesTrie<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoragesTrie")
            .field("address", &self.address)
            .field("tree", &self.tree)
            .field("storage", &self.storage)
            .finish()
    }
}

impl<S: Storage> StoragesTrie<S> {
    pub fn new(storage: S, address: ContractAddress, root: TrieNodeIndex) -> Self {
        Self { address, tree: MerkleTree::new(root), storage }
    }

    pub fn empty(storage: S, address: ContractAddress) -> Self {
        Self { address, tree: MerkleTree::empty(), storage }
    }

    pub fn address(&self) -> ContractAddress {
        self.address
    }

    pub fn insert(
        &mut self,
        storage_key: StorageKey,
        storage_value: StorageValue,
    ) -> anyhow::Result<()> {
        let key = storage_key.to_bytes_be().as_bits::<bitvec::order::Msb0>()[5..].to_owned();
        self.tree.set(&self.storage, key, storage_value)
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
        storage_keys: &[StorageKey],
    ) -> anyhow::Result<Vec<Vec<(ProofNode, Felt)>>> {
        let bit_keys: Vec<_> = storage_keys
            .iter()
            .map(|k| k.to_bytes_be().as_bits::<bitvec::order::Msb0>()[5..].to_owned())
            .collect();
        let refs: Vec<_> = bit_keys.iter().map(|k| k.as_bitslice()).collect();
        MerkleTree::<Pedersen, 251>::get_proofs(root, &self.storage, &refs)
    }
}
