use bitvec::order::Msb0;
use bitvec::slice::BitSlice;
use katana_primitives::Felt;

use crate::node::{StoredNode, TrieNodeIndex};

/// Trait for reading trie data from persistent storage.
pub trait Storage {
    /// Gets the stored node at the given index.
    fn get(&self, index: TrieNodeIndex) -> anyhow::Result<Option<StoredNode>>;

    /// Gets the hash of the node at the given index.
    fn hash(&self, index: TrieNodeIndex) -> anyhow::Result<Option<Felt>>;

    /// Gets the leaf value at the given path.
    fn leaf(&self, path: &BitSlice<u8, Msb0>) -> anyhow::Result<Option<Felt>>;
}
