use std::collections::HashMap;

use katana_primitives::Felt;

use crate::node::{StoredNode, TrieNodeIndex};
use crate::storage::Storage;

/// In-memory storage implementation for testing and ephemeral use (e.g., genesis).
///
/// Uses a HashMap so it can hold nodes at arbitrary indices — both sequentially
/// allocated (from `apply_update`) and at existing DB indices (when loading an
/// existing trie into memory).
#[derive(Debug, Default)]
pub struct MemStorage {
    nodes: HashMap<u64, (Felt, StoredNode)>,
    next_index: u64,
}

impl MemStorage {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the next available index.
    pub fn next_index(&self) -> TrieNodeIndex {
        TrieNodeIndex(self.next_index)
    }

    /// Inserts a node at a specific index. Used for bulk-loading from DB.
    pub fn insert_node(&mut self, index: u64, hash: Felt, node: StoredNode) {
        self.nodes.insert(index, (hash, node));
        // Keep next_index past any inserted index
        if index >= self.next_index {
            self.next_index = index + 1;
        }
    }

    /// Applies a [`TrieUpdate`](crate::node::TrieUpdate) to this storage, returning the root
    /// index.
    pub fn apply_update(&mut self, update: &crate::node::TrieUpdate) -> Option<TrieNodeIndex> {
        if update.nodes_added.is_empty() {
            return None;
        }

        let base = self.next_index;
        for (i, (hash, node)) in update.nodes_added.iter().enumerate() {
            let index = base + i as u64;
            let stored = resolve_node(node, base);
            self.nodes.insert(index, (*hash, stored));
        }

        self.next_index = base + update.nodes_added.len() as u64;

        Some(TrieNodeIndex(self.next_index - 1))
    }
}

impl Storage for MemStorage {
    fn get(&self, index: TrieNodeIndex) -> anyhow::Result<Option<StoredNode>> {
        Ok(self.nodes.get(&index.0).map(|(_, n)| n.clone()))
    }

    fn hash(&self, index: TrieNodeIndex) -> anyhow::Result<Option<Felt>> {
        Ok(self.nodes.get(&index.0).map(|(h, _)| *h))
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
        Node::LeafBinary { left_hash, right_hash } => {
            StoredNode::LeafBinary { left_hash: *left_hash, right_hash: *right_hash }
        }
        Node::LeafEdge { path, child_hash } => {
            StoredNode::LeafEdge { path: path.clone(), child_hash: *child_hash }
        }
    }
}
