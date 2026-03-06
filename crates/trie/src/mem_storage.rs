use std::collections::HashMap;

use bitvec::order::Msb0;
use bitvec::prelude::BitVec;
use bitvec::slice::BitSlice;
use katana_primitives::Felt;

use crate::node::{StoredNode, TrieNodeIndex};
use crate::storage::Storage;

/// In-memory storage implementation for testing and ephemeral use (e.g., genesis).
#[derive(Debug, Default)]
pub struct MemStorage {
    nodes: Vec<(Felt, StoredNode)>,
    leaves: HashMap<BitVec<u8, Msb0>, Felt>,
}

impl MemStorage {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the next available index.
    pub fn next_index(&self) -> TrieNodeIndex {
        TrieNodeIndex(self.nodes.len() as u64)
    }

    /// Applies a [`TrieUpdate`](crate::node::TrieUpdate) to this storage, returning the root
    /// index.
    pub fn apply_update(&mut self, update: &crate::node::TrieUpdate) -> Option<TrieNodeIndex> {
        if update.nodes_added.is_empty() {
            return None;
        }

        let base = self.nodes.len() as u64;
        for (hash, node) in &update.nodes_added {
            let stored = resolve_node(node, base);
            self.nodes.push((*hash, stored));
        }

        Some(TrieNodeIndex(base + update.nodes_added.len() as u64 - 1))
    }

    /// Sets a leaf value for the given path.
    pub fn set_leaf(&mut self, path: BitVec<u8, Msb0>, value: Felt) {
        if value == Felt::ZERO {
            self.leaves.remove(&path);
        } else {
            self.leaves.insert(path, value);
        }
    }
}

impl Storage for MemStorage {
    fn get(&self, index: TrieNodeIndex) -> anyhow::Result<Option<StoredNode>> {
        Ok(self.nodes.get(index.0 as usize).map(|(_, n)| n.clone()))
    }

    fn hash(&self, index: TrieNodeIndex) -> anyhow::Result<Option<Felt>> {
        Ok(self.nodes.get(index.0 as usize).map(|(h, _)| *h))
    }

    fn leaf(&self, path: &BitSlice<u8, Msb0>) -> anyhow::Result<Option<Felt>> {
        Ok(self.leaves.get(path).copied())
    }
}

/// Resolves a [crate::node::Node] with [crate::node::NodeRef] children into a [StoredNode] with
/// concrete indices.
fn resolve_node(node: &crate::node::Node, base: u64) -> StoredNode {
    use crate::node::{Node, NodeRef};

    fn resolve_ref(r: &NodeRef, base: u64) -> TrieNodeIndex {
        match r {
            NodeRef::StorageIndex(idx) => *idx,
            NodeRef::Index(i) => TrieNodeIndex(base + *i as u64),
        }
    }

    match node {
        Node::Binary { left, right } => {
            StoredNode::Binary { left: resolve_ref(left, base), right: resolve_ref(right, base) }
        }
        Node::Edge { child, path } => {
            StoredNode::Edge { child: resolve_ref(child, base), path: path.clone() }
        }
        Node::LeafBinary => StoredNode::LeafBinary,
        Node::LeafEdge { path } => StoredNode::LeafEdge { path: path.clone() },
    }
}
