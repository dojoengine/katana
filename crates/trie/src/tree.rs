use std::cell::RefCell;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::rc::Rc;

use anyhow::Context;
use bitvec::order::Msb0;
use bitvec::prelude::BitVec;
use bitvec::slice::BitSlice;
use katana_primitives::Felt;
use starknet_types_core::hash::StarkHash;

use crate::node::*;
use crate::proof::ProofNode;
use crate::storage::Storage;

/// A Binary Merkle-Patricia Tree.
///
/// Ported from pathfinder's merkle-tree crate. Uses an immutable tree model
/// where mutations are tracked in-memory and materialized via `commit()`.
///
/// Leaf hashes are embedded directly in leaf nodes (`InternalNode::Leaf(hash)`)
/// and persisted in the parent `LeafEdge`/`LeafBinary` stored nodes. This
/// eliminates the need for a separate leaf table and enables correct historical
/// trie access.
pub struct MerkleTree<H: StarkHash, const HEIGHT: usize> {
    root: Option<Rc<RefCell<InternalNode>>>,
    nodes_removed: Vec<TrieNodeIndex>,
    _hasher: PhantomData<H>,
}

impl<H: StarkHash, const HEIGHT: usize> std::fmt::Debug for MerkleTree<H, HEIGHT> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MerkleTree").field("root", &self.root).finish()
    }
}

impl<H: StarkHash, const HEIGHT: usize> Clone for MerkleTree<H, HEIGHT> {
    fn clone(&self) -> Self {
        Self {
            root: self.root.clone(),
            nodes_removed: self.nodes_removed.clone(),
            _hasher: PhantomData,
        }
    }
}

impl<H: StarkHash, const HEIGHT: usize> MerkleTree<H, HEIGHT> {
    /// Creates a tree with an existing root node from storage.
    pub fn new(root: TrieNodeIndex) -> Self {
        let root = Some(Rc::new(RefCell::new(InternalNode::Unresolved(root))));
        Self { root, _hasher: PhantomData, nodes_removed: Default::default() }
    }

    /// Creates an empty tree.
    pub fn empty() -> Self {
        Self { root: None, _hasher: PhantomData, nodes_removed: Default::default() }
    }

    /// Sets a key-value pair. Setting value to ZERO deletes the leaf.
    pub fn set(
        &mut self,
        storage: &impl Storage,
        key: BitVec<u8, Msb0>,
        value: Felt,
    ) -> anyhow::Result<()> {
        if value == Felt::ZERO {
            return self.delete_leaf(storage, &key);
        }

        let path = self.traverse(storage, &key)?;

        match path.last() {
            Some(node) => {
                let updated = match &*node.borrow() {
                    InternalNode::Edge(edge) => {
                        let common = edge.common_path(&key);
                        let branch_height = edge.height + common.len();
                        let child_height = branch_height + 1;
                        let new_path = key[child_height..].to_bitvec();
                        let old_path = edge.path[common.len() + 1..].to_bitvec();

                        let new = match new_path.is_empty() {
                            true => Rc::new(RefCell::new(InternalNode::Leaf(value))),
                            false => {
                                let new_edge = InternalNode::Edge(EdgeNode {
                                    storage_index: None,
                                    height: child_height,
                                    path: new_path,
                                    child: Rc::new(RefCell::new(InternalNode::Leaf(value))),
                                });
                                Rc::new(RefCell::new(new_edge))
                            }
                        };

                        let old = match old_path.is_empty() {
                            true => edge.child.clone(),
                            false => {
                                let old_edge = InternalNode::Edge(EdgeNode {
                                    storage_index: None,
                                    height: child_height,
                                    path: old_path,
                                    child: edge.child.clone(),
                                });
                                Rc::new(RefCell::new(old_edge))
                            }
                        };

                        let new_direction = Direction::from(key[branch_height]);
                        let (left, right) = match new_direction {
                            Direction::Left => (new, old),
                            Direction::Right => (old, new),
                        };

                        let branch = InternalNode::Binary(BinaryNode {
                            storage_index: None,
                            height: branch_height,
                            left,
                            right,
                        });

                        match common.is_empty() {
                            true => branch,
                            false => InternalNode::Edge(EdgeNode {
                                storage_index: None,
                                height: edge.height,
                                path: common.to_bitvec(),
                                child: Rc::new(RefCell::new(branch)),
                            }),
                        }
                    }
                    // Updating an existing leaf — replace with new value.
                    InternalNode::Leaf(_) => InternalNode::Leaf(value),
                    InternalNode::Unresolved(_) | InternalNode::Binary(_) => {
                        unreachable!("End of traversal cannot be unresolved or binary")
                    }
                };

                let old_node = node.replace(updated);
                if let Some(index) = old_node.storage_index() {
                    self.nodes_removed.push(index);
                };
            }
            None => {
                let edge = InternalNode::Edge(EdgeNode {
                    storage_index: None,
                    height: 0,
                    path: key.to_bitvec(),
                    child: Rc::new(RefCell::new(InternalNode::Leaf(value))),
                });
                self.root = Some(Rc::new(RefCell::new(edge)));
            }
        }

        Ok(())
    }

    /// Looks up a value by key.
    pub fn get(
        &self,
        storage: &impl Storage,
        key: BitVec<u8, Msb0>,
    ) -> anyhow::Result<Option<Felt>> {
        let nodes = self.traverse(storage, &key)?;
        let Some(node) = nodes.last() else {
            return Ok(None);
        };

        let result =
            if let InternalNode::Leaf(hash) = &*node.borrow() { Some(*hash) } else { None };

        Ok(result)
    }

    /// Deletes a leaf from the tree.
    fn delete_leaf(
        &mut self,
        storage: &impl Storage,
        key: &BitSlice<u8, Msb0>,
    ) -> anyhow::Result<()> {
        let path = self.traverse(storage, key)?;

        match path.last() {
            Some(node) => match &*node.borrow() {
                InternalNode::Leaf(_) => {}
                _ => return Ok(()),
            },
            None => return Ok(()),
        }

        let mut indexes_removed = Vec::new();
        let mut node_iter = path.into_iter().rev().skip_while(|node| {
            let node = node.borrow();
            match *node {
                InternalNode::Binary(_) => false,
                _ => {
                    if let Some(index) = node.storage_index() {
                        indexes_removed.push(index);
                    };
                    true
                }
            }
        });

        match node_iter.next() {
            Some(node) => {
                let new_edge = {
                    let binary = node.borrow().as_binary().cloned().unwrap();
                    let direction = binary.direction(key).invert();
                    let child = binary.get_child(direction);
                    let path = std::iter::once(bool::from(direction)).collect::<BitVec<_, _>>();
                    let mut edge =
                        EdgeNode { storage_index: None, height: binary.height, path, child };

                    self.merge_edges(storage, &mut edge)?;
                    edge
                };
                let old_node = node.replace(InternalNode::Edge(new_edge));
                if let Some(index) = old_node.storage_index() {
                    self.nodes_removed.push(index);
                };
            }
            None => {
                self.root = None;
                self.nodes_removed.extend(indexes_removed);
                return Ok(());
            }
        };

        if let Some(node) = node_iter.next() {
            if let InternalNode::Edge(edge) = &mut *node.borrow_mut() {
                self.merge_edges(storage, edge)?;
            }
        }

        self.nodes_removed.extend(indexes_removed);
        Ok(())
    }

    /// Commits all mutations and returns the update to be persisted.
    pub fn commit(self, storage: &impl Storage) -> anyhow::Result<TrieUpdate> {
        let mut added = Vec::new();
        let mut removed = Vec::new();

        let root_hash = if let Some(root) = self.root.as_ref() {
            match &mut *root.borrow_mut() {
                InternalNode::Unresolved(idx) => storage
                    .hash(*idx)
                    .context("Fetching root node's hash")?
                    .context("Root node's hash is missing")?,
                other => {
                    let (root_hash, _) =
                        Self::commit_subtree(other, &mut added, &mut removed, storage)?;
                    root_hash
                }
            }
        } else {
            Felt::ZERO
        };

        removed.extend(self.nodes_removed);

        Ok(TrieUpdate { nodes_added: added, nodes_removed: removed, root_commitment: root_hash })
    }

    fn commit_subtree(
        node: &mut InternalNode,
        added: &mut Vec<(Felt, Node)>,
        removed: &mut Vec<TrieNodeIndex>,
        storage: &impl Storage,
    ) -> anyhow::Result<(Felt, Option<NodeRef>)> {
        let result = match node {
            InternalNode::Unresolved(idx) => {
                let hash = storage
                    .hash(*idx)
                    .context("Fetching stored node's hash")?
                    .context("Stored node's hash is missing")?;
                (hash, Some(NodeRef::StorageIndex(*idx)))
            }
            InternalNode::Leaf(hash) => (*hash, None),
            InternalNode::Binary(binary) => {
                let (left_hash, left_child) =
                    Self::commit_subtree(&mut binary.left.borrow_mut(), added, removed, storage)?;
                let (right_hash, right_child) =
                    Self::commit_subtree(&mut binary.right.borrow_mut(), added, removed, storage)?;
                let hash = BinaryNode::calculate_hash::<H>(left_hash, right_hash);

                let persisted_node = match (left_child, right_child) {
                    (None, None) => Node::LeafBinary { left_hash, right_hash },
                    (Some(_), None) | (None, Some(_)) => {
                        anyhow::bail!("Inconsistent binary children. Both must be leaves or not.")
                    }
                    (Some(left), Some(right)) => Node::Binary { left, right },
                };

                if let Some(storage_index) = binary.storage_index {
                    removed.push(storage_index);
                };

                let node_index = added.len();
                added.push((hash, persisted_node));

                (hash, Some(NodeRef::Index(node_index)))
            }
            InternalNode::Edge(edge) => {
                let (child_hash, child) =
                    Self::commit_subtree(&mut edge.child.borrow_mut(), added, removed, storage)?;

                let hash = EdgeNode::calculate_hash::<H>(child_hash, &edge.path);

                let persisted_node = match child {
                    None => Node::LeafEdge { path: edge.path.clone(), child_hash },
                    Some(child) => Node::Edge { child, path: edge.path.clone() },
                };

                let node_index = added.len();
                added.push((hash, persisted_node));
                if let Some(storage_index) = edge.storage_index {
                    removed.push(storage_index);
                };

                (hash, Some(NodeRef::Index(node_index)))
            }
        };

        Ok(result)
    }

    /// Traverses the tree towards the given key, returning the path of nodes encountered.
    fn traverse(
        &self,
        storage: &impl Storage,
        dst: &BitSlice<u8, Msb0>,
    ) -> anyhow::Result<Vec<Rc<RefCell<InternalNode>>>> {
        let Some(mut current) = self.root.clone() else {
            return Ok(Vec::new());
        };

        let mut height = 0;
        let mut nodes = Vec::new();
        loop {
            let current_tmp = current.borrow().clone();

            let next = match current_tmp {
                InternalNode::Unresolved(idx) => {
                    let node = self.resolve(storage, idx, height)?;
                    current.replace(node);
                    current
                }
                InternalNode::Binary(binary) => {
                    nodes.push(current.clone());
                    let next = binary.direction(dst);
                    let next = binary.get_child(next);
                    height += 1;
                    next
                }
                InternalNode::Edge(edge) if edge.path_matches(dst) => {
                    nodes.push(current.clone());
                    height += edge.path.len();
                    edge.child.clone()
                }
                InternalNode::Leaf(_) | InternalNode::Edge(_) => {
                    nodes.push(current);
                    return Ok(nodes);
                }
            };

            current = next;
        }
    }

    /// Resolves an unresolved node from storage.
    fn resolve(
        &self,
        storage: &impl Storage,
        index: TrieNodeIndex,
        height: usize,
    ) -> anyhow::Result<InternalNode> {
        anyhow::ensure!(
            height < HEIGHT,
            "Attempted to resolve node at height {height} exceeding tree height {HEIGHT}"
        );

        let node = storage
            .get(index)?
            .with_context(|| format!("Node {index} at height {height} is missing"))?;

        let node = match node {
            StoredNode::Binary { left, right } => InternalNode::Binary(BinaryNode {
                storage_index: Some(index),
                height,
                left: Rc::new(RefCell::new(InternalNode::Unresolved(left))),
                right: Rc::new(RefCell::new(InternalNode::Unresolved(right))),
            }),
            StoredNode::Edge { child, path } => InternalNode::Edge(EdgeNode {
                storage_index: Some(index),
                height,
                path,
                child: Rc::new(RefCell::new(InternalNode::Unresolved(child))),
            }),
            StoredNode::LeafBinary { left_hash, right_hash } => InternalNode::Binary(BinaryNode {
                storage_index: Some(index),
                height,
                left: Rc::new(RefCell::new(InternalNode::Leaf(left_hash))),
                right: Rc::new(RefCell::new(InternalNode::Leaf(right_hash))),
            }),
            StoredNode::LeafEdge { path, child_hash } => InternalNode::Edge(EdgeNode {
                storage_index: Some(index),
                height,
                path,
                child: Rc::new(RefCell::new(InternalNode::Leaf(child_hash))),
            }),
        };

        Ok(node)
    }

    /// Merges consecutive edge nodes.
    fn merge_edges(&mut self, storage: &impl Storage, parent: &mut EdgeNode) -> anyhow::Result<()> {
        let resolved_child = match &*parent.child.borrow() {
            InternalNode::Unresolved(hash) => {
                self.resolve(storage, *hash, parent.height + parent.path.len())?
            }
            other => other.clone(),
        };

        if let Some(child_edge) = resolved_child.as_edge().cloned() {
            parent.path.extend_from_bitslice(&child_edge.path);
            if let Some(storage_index) = child_edge.storage_index {
                self.nodes_removed.push(storage_index);
            }
            parent.child = child_edge.child;
        }

        Ok(())
    }

    /// Generates Merkle proofs for the given keys against a committed tree.
    pub fn get_proofs(
        root: TrieNodeIndex,
        storage: &impl Storage,
        keys: &[&BitSlice<u8, Msb0>],
    ) -> anyhow::Result<Vec<Vec<(ProofNode, Felt)>>> {
        let mut node_cache: HashMap<TrieNodeIndex, StoredNode> = HashMap::new();
        let mut node_hash_cache: HashMap<TrieNodeIndex, Felt> = HashMap::new();
        let mut proofs = vec![];

        for key in keys {
            let mut nodes = Vec::new();
            let mut next = Some(root);
            let mut height = 0;

            while let Some(index) = next.take() {
                let node = match node_cache.get(&index) {
                    Some(node) => node.clone(),
                    None => {
                        let Some(node) = storage.get(index).context("Resolving node")? else {
                            anyhow::bail!("Storage node {index} is missing");
                        };
                        node_cache.insert(index, node.clone());
                        node
                    }
                };

                let proof_node = match node {
                    StoredNode::Binary { left, right } => {
                        next = match key.get(height).map(|b| Direction::from(*b)) {
                            Some(Direction::Left) => Some(left),
                            Some(Direction::Right) => Some(right),
                            None => {
                                anyhow::bail!("Key path too short for binary node")
                            }
                        };
                        height += 1;

                        let left = storage
                            .hash(left)
                            .context("Querying left child's hash")?
                            .context("Left child's hash is missing")?;
                        let right = storage
                            .hash(right)
                            .context("Querying right child's hash")?
                            .context("Right child's hash is missing")?;

                        ProofNode::Binary { left, right }
                    }
                    StoredNode::Edge { child, path } => {
                        let key_slice = key
                            .get(height..height + path.len())
                            .context("Key path is too short for edge node")?;
                        height += path.len();

                        if key_slice == path {
                            next = Some(child);
                        }

                        let child = storage
                            .hash(child)
                            .context("Querying child's hash")?
                            .context("Child's hash is missing")?;

                        ProofNode::Edge { child, path }
                    }
                    StoredNode::LeafBinary { left_hash, right_hash } => {
                        ProofNode::Binary { left: left_hash, right: right_hash }
                    }
                    StoredNode::LeafEdge { path, child_hash } => {
                        ProofNode::Edge { child: child_hash, path }
                    }
                };

                let node_hash = match node_hash_cache.get(&index) {
                    Some(&hash) => hash,
                    None => {
                        let hash = storage
                            .hash(index)
                            .context("Querying node hash")?
                            .context("Node hash is missing")?;
                        node_hash_cache.insert(index, hash);
                        hash
                    }
                };
                nodes.push((proof_node, node_hash));
            }

            proofs.push(nodes);
        }

        Ok(proofs)
    }
}
