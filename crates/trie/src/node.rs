use std::cell::RefCell;
use std::rc::Rc;

use bitvec::order::Msb0;
use bitvec::prelude::BitVec;
use bitvec::slice::BitSlice;
use katana_primitives::Felt;
use starknet_types_core::hash::StarkHash;

/// Index into persisted trie node storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TrieNodeIndex(pub u64);

impl From<u64> for TrieNodeIndex {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl std::fmt::Display for TrieNodeIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// In-memory node during tree mutation.
#[derive(Clone, Debug, PartialEq)]
pub enum InternalNode {
    /// A node that has not been fetched from storage yet.
    Unresolved(TrieNodeIndex),
    /// A branch node with exactly two children.
    Binary(BinaryNode),
    /// Describes a path connecting two other nodes.
    Edge(EdgeNode),
    /// A leaf node.
    Leaf,
}

/// Describes the [InternalNode::Binary] variant.
#[derive(Clone, Debug, PartialEq)]
pub struct BinaryNode {
    /// The storage index of this node (if it was loaded from storage).
    pub storage_index: Option<TrieNodeIndex>,
    /// The height of this node in the tree.
    pub height: usize,
    /// Left child.
    pub left: Rc<RefCell<InternalNode>>,
    /// Right child.
    pub right: Rc<RefCell<InternalNode>>,
}

/// Describes the [InternalNode::Edge] variant.
#[derive(Clone, Debug, PartialEq)]
pub struct EdgeNode {
    /// The storage index of this node (if it was loaded from storage).
    pub storage_index: Option<TrieNodeIndex>,
    /// The starting height of this node in the tree.
    pub height: usize,
    /// The path this edge takes.
    pub path: BitVec<u8, Msb0>,
    /// The child of this node.
    pub child: Rc<RefCell<InternalNode>>,
}

/// Describes the direction a child of a [BinaryNode] may have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
}

impl Direction {
    pub fn invert(self) -> Direction {
        match self {
            Direction::Left => Direction::Right,
            Direction::Right => Direction::Left,
        }
    }
}

impl From<bool> for Direction {
    fn from(tf: bool) -> Self {
        match tf {
            true => Direction::Right,
            false => Direction::Left,
        }
    }
}

impl From<Direction> for bool {
    fn from(direction: Direction) -> Self {
        match direction {
            Direction::Left => false,
            Direction::Right => true,
        }
    }
}

impl BinaryNode {
    /// Maps the key's bit at the binary node's height to a [Direction].
    pub fn direction(&self, key: &BitSlice<u8, Msb0>) -> Direction {
        key[self.height].into()
    }

    /// Returns the left or right child.
    pub fn get_child(&self, direction: Direction) -> Rc<RefCell<InternalNode>> {
        match direction {
            Direction::Left => self.left.clone(),
            Direction::Right => self.right.clone(),
        }
    }

    pub(crate) fn calculate_hash<H: StarkHash>(left: Felt, right: Felt) -> Felt {
        H::hash(&left, &right)
    }
}

impl InternalNode {
    pub fn as_binary(&self) -> Option<&BinaryNode> {
        match self {
            InternalNode::Binary(binary) => Some(binary),
            _ => None,
        }
    }

    pub fn as_edge(&self) -> Option<&EdgeNode> {
        match self {
            InternalNode::Edge(edge) => Some(edge),
            _ => None,
        }
    }

    pub fn storage_index(&self) -> Option<TrieNodeIndex> {
        match self {
            InternalNode::Unresolved(storage_index) => Some(*storage_index),
            InternalNode::Binary(binary) => binary.storage_index,
            InternalNode::Edge(edge) => edge.storage_index,
            InternalNode::Leaf => None,
        }
    }
}

impl EdgeNode {
    /// Returns true if the edge node's path matches the same path given by the key.
    pub fn path_matches(&self, key: &BitSlice<u8, Msb0>) -> bool {
        self.path == key[self.height..self.height + self.path.len()]
    }

    /// Returns the common bit prefix between the edge node's path and the given key.
    pub fn common_path(&self, key: &BitSlice<u8, Msb0>) -> &BitSlice<u8, Msb0> {
        let key_path = key.iter().skip(self.height);
        let common_length = key_path.zip(self.path.iter()).take_while(|(a, b)| a == b).count();

        &self.path[..common_length]
    }

    pub(crate) fn calculate_hash<H: StarkHash>(child: Felt, path: &BitSlice<u8, Msb0>) -> Felt {
        let mut length = [0; 32];
        // Safe as len() is guaranteed to be <= 251
        length[31] = path.len() as u8;
        let length = Felt::from_bytes_be(&length);
        let path = felt_from_bits(path);

        H::hash(&child, &path) + length
    }
}

/// Converts a bit slice to a Felt value.
pub fn felt_from_bits(bits: &BitSlice<u8, Msb0>) -> Felt {
    let mut bytes = [0u8; 32];
    let target = &mut bitvec::view::BitViewSized::into_bitarray::<Msb0>(bytes);
    target[256 - bits.len()..].copy_from_bitslice(bits);
    bytes = target.into_inner();
    Felt::from_bytes_be(&bytes)
}

/// Node as persisted in DB (no Rc/RefCell).
#[derive(Debug, Clone, PartialEq)]
pub enum StoredNode {
    Binary { left: TrieNodeIndex, right: TrieNodeIndex },
    Edge { child: TrieNodeIndex, path: BitVec<u8, Msb0> },
    LeafBinary,
    LeafEdge { path: BitVec<u8, Msb0> },
}

/// Reference to a child node in a TrieUpdate.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeRef {
    StorageIndex(TrieNodeIndex),
    /// Index into [TrieUpdate::nodes_added].
    Index(usize),
}

/// Persisted node using [NodeRef] for children.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Binary { left: NodeRef, right: NodeRef },
    Edge { child: NodeRef, path: BitVec<u8, Msb0> },
    LeafBinary,
    LeafEdge { path: BitVec<u8, Msb0> },
}

/// Result of committing tree mutations.
#[derive(Debug, Clone, Default)]
pub struct TrieUpdate {
    pub nodes_added: Vec<(Felt, Node)>,
    pub nodes_removed: Vec<TrieNodeIndex>,
    pub root_commitment: Felt,
    /// Leaf values that were set during this update.
    /// Maps trie path (as BitVec) to the leaf value (hash).
    pub leaves: std::collections::HashMap<BitVec<u8, Msb0>, Felt>,
}
