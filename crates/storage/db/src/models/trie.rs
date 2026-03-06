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

/// Helper: serialize a BitVec path with alignment normalization.
fn write_path(buf: &mut Vec<u8>, path: &BitVec<u8, Msb0>) {
    // Normalize alignment: as_raw_slice() includes head offset bits from
    // BitVec slicing, so we copy logical bits into a fresh aligned BitVec.
    let mut aligned = BitVec::<u8, Msb0>::new();
    aligned.extend_from_bitslice(path);
    buf.extend_from_slice(&(path.len() as u16).to_be_bytes());
    buf.extend_from_slice(aligned.as_raw_slice());
}

/// Helper: deserialize a BitVec path.
fn read_path(bytes: &[u8]) -> Result<BitVec<u8, Msb0>, CodecError> {
    if bytes.len() < 2 {
        return Err(CodecError::Decode("Path data too short".into()));
    }
    let path_len = u16::from_be_bytes(bytes[..2].try_into().unwrap()) as usize;
    let path_bytes = &bytes[2..];
    let mut path = BitVec::<u8, Msb0>::from_slice(path_bytes);
    path.truncate(path_len);
    Ok(path)
}

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
                write_path(&mut buf, path);
            }
            StoredNode::LeafBinary { left_hash, right_hash } => {
                buf.push(TAG_LEAF_BINARY);
                buf.extend_from_slice(&left_hash.to_bytes_be());
                buf.extend_from_slice(&right_hash.to_bytes_be());
            }
            StoredNode::LeafEdge { path, child_hash } => {
                buf.push(TAG_LEAF_EDGE);
                buf.extend_from_slice(&child_hash.to_bytes_be());
                write_path(&mut buf, path);
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
                let path = read_path(&bytes[41..])?;
                StoredNode::Edge { child: TrieNodeIndex(child), path }
            }
            TAG_LEAF_BINARY => {
                if bytes.len() < 33 + 64 {
                    return Err(CodecError::Decode("LeafBinary node too short".into()));
                }
                let left_hash = Felt::from_bytes_be_slice(&bytes[33..65]);
                let right_hash = Felt::from_bytes_be_slice(&bytes[65..97]);
                StoredNode::LeafBinary { left_hash, right_hash }
            }
            TAG_LEAF_EDGE => {
                if bytes.len() < 33 + 32 + 2 {
                    return Err(CodecError::Decode("LeafEdge node too short".into()));
                }
                let child_hash = Felt::from_bytes_be_slice(&bytes[33..65]);
                let path = read_path(&bytes[65..])?;
                StoredNode::LeafEdge { path, child_hash }
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
        let entry = TrieNodeEntry {
            hash: Felt::from(0x5555u64),
            node: StoredNode::LeafBinary {
                left_hash: Felt::from(100u64),
                right_hash: Felt::from(200u64),
            },
        };
        let compressed = entry.clone().compress().unwrap();
        let decompressed = TrieNodeEntry::decompress(&compressed).unwrap();
        assert_eq!(entry, decompressed);
    }

    #[test]
    fn trie_node_entry_leaf_edge_roundtrip() {
        use katana_trie::bitvec::prelude::*;

        let path = katana_trie::bitvec::bitvec![u8, Msb0; 0, 1, 0, 1, 0, 1, 0, 1];
        let entry = TrieNodeEntry {
            hash: Felt::from(0x9999u64),
            node: StoredNode::LeafEdge { path, child_hash: Felt::from(42u64) },
        };
        let compressed = entry.clone().compress().unwrap();
        let decompressed = TrieNodeEntry::decompress(&compressed).unwrap();
        assert_eq!(entry, decompressed);
    }

    /// Test that edge paths with non-zero head offset (from BitVec slicing)
    /// roundtrip correctly through Compress/Decompress.
    #[test]
    fn trie_node_entry_edge_offset_bitvec_roundtrip() {
        use katana_trie::bitvec::prelude::*;

        let felt = Felt::from(0xDEADBEEFu64);
        let bits = felt.to_bytes_be();
        let full_bits = bits.as_bits::<Msb0>();
        let offset_path = full_bits[5..].to_owned();
        let edge_path = offset_path[..10].to_owned();

        let entry = TrieNodeEntry {
            hash: Felt::from(0x1111u64),
            node: StoredNode::Edge { child: TrieNodeIndex(7), path: edge_path.clone() },
        };
        let compressed = entry.clone().compress().unwrap();
        let decompressed = TrieNodeEntry::decompress(&compressed).unwrap();

        if let StoredNode::Edge { path: decompressed_path, .. } = &decompressed.node {
            assert_eq!(edge_path.len(), decompressed_path.len());
            assert_eq!(edge_path, *decompressed_path);
        } else {
            panic!("Expected Edge node");
        }
    }
}
