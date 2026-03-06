use katana_primitives::Felt;
use katana_trie::bitvec::order::Msb0;
use katana_trie::bitvec::prelude::BitVec;
use katana_trie::node::{StoredNode, TrieNodeIndex};

use crate::codecs::{Compress, Decompress};
use crate::error::CodecError;

/// A trie node entry as stored in the database.
/// Contains the stored node structure and its computed hash.
#[derive(Debug, Clone, PartialEq)]
pub struct TrieNodeEntry {
    pub hash: Felt,
    pub node: StoredNode,
}

/// Tag bytes for encoding StoredNode variants.
const TAG_BINARY: u8 = 0;
const TAG_EDGE: u8 = 1;
const TAG_LEAF_BINARY: u8 = 2;
const TAG_LEAF_EDGE: u8 = 3;

impl Compress for TrieNodeEntry {
    type Compressed = Vec<u8>;

    fn compress(self) -> Result<Self::Compressed, CodecError> {
        let mut buf = Vec::new();

        // Write hash (32 bytes)
        buf.extend_from_slice(&self.hash.to_bytes_be());

        // Write node
        match &self.node {
            StoredNode::Binary { left, right } => {
                buf.push(TAG_BINARY);
                buf.extend_from_slice(&left.0.to_be_bytes());
                buf.extend_from_slice(&right.0.to_be_bytes());
            }
            StoredNode::Edge { child, path } => {
                buf.push(TAG_EDGE);
                buf.extend_from_slice(&child.0.to_be_bytes());
                // Normalize alignment: as_raw_slice() includes head offset bits from
                // BitVec slicing, so we copy logical bits into a fresh aligned BitVec.
                let mut aligned = BitVec::<u8, Msb0>::new();
                aligned.extend_from_bitslice(path);
                buf.extend_from_slice(&(path.len() as u16).to_be_bytes());
                buf.extend_from_slice(aligned.as_raw_slice());
            }
            StoredNode::LeafBinary => {
                buf.push(TAG_LEAF_BINARY);
            }
            StoredNode::LeafEdge { path } => {
                buf.push(TAG_LEAF_EDGE);
                let mut aligned = BitVec::<u8, Msb0>::new();
                aligned.extend_from_bitslice(path);
                buf.extend_from_slice(&(path.len() as u16).to_be_bytes());
                buf.extend_from_slice(aligned.as_raw_slice());
            }
        }

        Ok(buf)
    }
}

impl Decompress for TrieNodeEntry {
    fn decompress<B: AsRef<[u8]>>(bytes: B) -> Result<Self, CodecError> {
        let bytes = bytes.as_ref();

        if bytes.len() < 33 {
            return Err(CodecError::Decode("TrieNodeEntry too short".into()));
        }

        let hash = Felt::from_bytes_be_slice(&bytes[..32]);
        let tag = bytes[32];

        let node = match tag {
            TAG_BINARY => {
                if bytes.len() < 33 + 16 {
                    return Err(CodecError::Decode("Binary node too short".into()));
                }
                let left = u64::from_be_bytes(bytes[33..41].try_into().unwrap());
                let right = u64::from_be_bytes(bytes[41..49].try_into().unwrap());
                StoredNode::Binary { left: TrieNodeIndex(left), right: TrieNodeIndex(right) }
            }
            TAG_EDGE => {
                if bytes.len() < 33 + 8 + 2 {
                    return Err(CodecError::Decode("Edge node too short".into()));
                }
                let child = u64::from_be_bytes(bytes[33..41].try_into().unwrap());
                let path_len = u16::from_be_bytes(bytes[41..43].try_into().unwrap()) as usize;
                let path_bytes = &bytes[43..];
                let mut path = BitVec::<u8, Msb0>::from_slice(path_bytes);
                path.truncate(path_len);
                StoredNode::Edge { child: TrieNodeIndex(child), path }
            }
            TAG_LEAF_BINARY => StoredNode::LeafBinary,
            TAG_LEAF_EDGE => {
                if bytes.len() < 33 + 2 {
                    return Err(CodecError::Decode("LeafEdge node too short".into()));
                }
                let path_len = u16::from_be_bytes(bytes[33..35].try_into().unwrap()) as usize;
                let path_bytes = &bytes[35..];
                let mut path = BitVec::<u8, Msb0>::from_slice(path_bytes);
                path.truncate(path_len);
                StoredNode::LeafEdge { path }
            }
            other => {
                return Err(CodecError::Decode(format!("Unknown node tag: {other}")));
            }
        };

        Ok(TrieNodeEntry { hash, node })
    }
}

/// Trie type discriminator for composite keys in TrieRoots and TrieBlockLog.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrieType {
    Classes = 0,
    Contracts = 1,
    Storage = 2,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trie_node_entry_binary_roundtrip() {
        let entry = TrieNodeEntry {
            hash: Felt::from(0x1234u64),
            node: StoredNode::Binary { left: TrieNodeIndex(10), right: TrieNodeIndex(20) },
        };
        let compressed = entry.clone().compress().unwrap();
        let decompressed = TrieNodeEntry::decompress(&compressed).unwrap();
        assert_eq!(entry, decompressed);
    }

    #[test]
    fn trie_node_entry_edge_roundtrip() {
        use katana_trie::bitvec::prelude::*;

        let path = katana_trie::bitvec::bitvec![u8, Msb0; 1, 0, 1, 1, 0];
        let entry = TrieNodeEntry {
            hash: Felt::from(0xABCDu64),
            node: StoredNode::Edge { child: TrieNodeIndex(42), path },
        };
        let compressed = entry.clone().compress().unwrap();
        let decompressed = TrieNodeEntry::decompress(&compressed).unwrap();
        assert_eq!(entry, decompressed);
    }

    #[test]
    fn trie_node_entry_leaf_binary_roundtrip() {
        let entry = TrieNodeEntry { hash: Felt::from(0x5555u64), node: StoredNode::LeafBinary };
        let compressed = entry.clone().compress().unwrap();
        let decompressed = TrieNodeEntry::decompress(&compressed).unwrap();
        assert_eq!(entry, decompressed);
    }

    #[test]
    fn trie_node_entry_leaf_edge_roundtrip() {
        use katana_trie::bitvec::prelude::*;

        let path = katana_trie::bitvec::bitvec![u8, Msb0; 0, 1, 0, 1, 0, 1, 0, 1];
        let entry =
            TrieNodeEntry { hash: Felt::from(0x9999u64), node: StoredNode::LeafEdge { path } };
        let compressed = entry.clone().compress().unwrap();
        let decompressed = TrieNodeEntry::decompress(&compressed).unwrap();
        assert_eq!(entry, decompressed);
    }

    /// Test that edge paths with non-zero head offset (from BitVec slicing)
    /// roundtrip correctly through Compress/Decompress.
    #[test]
    fn trie_node_entry_edge_offset_bitvec_roundtrip() {
        use katana_trie::bitvec::prelude::*;

        // Simulate the real-world case: hash.to_bytes_be().as_bits::<Msb0>()[5..].to_owned()
        // This creates a BitVec with a 5-bit head offset.
        let felt = Felt::from(0xDEADBEEFu64);
        let bits = felt.to_bytes_be();
        let full_bits = bits.as_bits::<Msb0>();
        let offset_path = full_bits[5..].to_owned(); // 251-bit path with 5-bit head offset

        // Take a sub-path (like an edge path in a trie)
        let edge_path = offset_path[..10].to_owned();

        let entry = TrieNodeEntry {
            hash: Felt::from(0x1111u64),
            node: StoredNode::Edge { child: TrieNodeIndex(7), path: edge_path.clone() },
        };
        let compressed = entry.clone().compress().unwrap();
        let decompressed = TrieNodeEntry::decompress(&compressed).unwrap();

        // The logical bits must match after roundtrip
        if let StoredNode::Edge { path: decompressed_path, .. } = &decompressed.node {
            assert_eq!(edge_path.len(), decompressed_path.len());
            assert_eq!(edge_path, *decompressed_path, "Edge path bits must match after roundtrip");
        } else {
            panic!("Expected Edge node");
        }
    }
}
