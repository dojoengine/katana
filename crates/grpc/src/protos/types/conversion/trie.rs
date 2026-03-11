//! Storage proof type conversions.

use tonic::Status;

use crate::protos::common::Felt as ProtoFelt;
use crate::protos::types::{
    BinaryNode, ClassesProof as ProtoClassesProof, ContractLeafData as ProtoContractLeafData,
    ContractStorageKeysRequest, ContractStorageProof,
    ContractStorageProofs as ProtoContractStorageProofs, ContractsProof as ProtoContractsProof,
    EdgeNode, GlobalRoots as ProtoGlobalRoots, MerkleProofNode, StorageProof as ProtoStorageProof,
};

// ============================================================
// Request type conversions (Proto → Internal)
// ============================================================

impl TryFrom<ContractStorageKeysRequest> for katana_rpc_types::trie::ContractStorageKeys {
    type Error = Status;

    fn try_from(proto: ContractStorageKeysRequest) -> Result<Self, Self::Error> {
        let address = proto
            .contract_address
            .ok_or_else(|| Status::invalid_argument("Missing contract_address"))?
            .try_into()?;

        let keys = proto.keys.into_iter().map(|f| f.try_into()).collect::<Result<Vec<_>, _>>()?;

        Ok(Self { address, keys })
    }
}

// ============================================================
// Response type conversions (Internal → Proto)
// ============================================================

impl From<katana_rpc_types::trie::GetStorageProofResponse> for ProtoStorageProof {
    fn from(response: katana_rpc_types::trie::GetStorageProofResponse) -> Self {
        ProtoStorageProof {
            global_roots: Some(response.global_roots.into()),
            classes_proof: Some(response.classes_proof.into()),
            contracts_proof: Some(response.contracts_proof.into()),
            contracts_storage_proofs: Some(response.contracts_storage_proofs.into()),
        }
    }
}

impl From<katana_rpc_types::trie::GlobalRoots> for ProtoGlobalRoots {
    fn from(roots: katana_rpc_types::trie::GlobalRoots) -> Self {
        ProtoGlobalRoots {
            block_hash: Some(roots.block_hash.into()),
            classes_tree_root: Some(roots.classes_tree_root.into()),
            contracts_tree_root: Some(roots.contracts_tree_root.into()),
        }
    }
}

impl From<katana_rpc_types::trie::ClassesProof> for ProtoClassesProof {
    fn from(proof: katana_rpc_types::trie::ClassesProof) -> Self {
        ProtoClassesProof { nodes: nodes_to_proto(proof.nodes) }
    }
}

impl From<katana_rpc_types::trie::ContractsProof> for ProtoContractsProof {
    fn from(proof: katana_rpc_types::trie::ContractsProof) -> Self {
        ProtoContractsProof {
            nodes: nodes_to_proto(proof.nodes),
            contract_leaves_data: proof
                .contract_leaves_data
                .into_iter()
                .map(ProtoContractLeafData::from)
                .collect(),
        }
    }
}

impl From<katana_rpc_types::trie::ContractLeafData> for ProtoContractLeafData {
    fn from(leaf: katana_rpc_types::trie::ContractLeafData) -> Self {
        ProtoContractLeafData {
            storage_root: Some(leaf.storage_root.into()),
            class_hash: Some(leaf.class_hash.into()),
            nonce: Some(leaf.nonce.into()),
        }
    }
}

impl From<katana_rpc_types::trie::ContractStorageProofs> for ProtoContractStorageProofs {
    fn from(proofs: katana_rpc_types::trie::ContractStorageProofs) -> Self {
        ProtoContractStorageProofs {
            proofs: proofs
                .nodes
                .into_iter()
                .map(|nodes| ContractStorageProof { nodes: nodes_to_proto(nodes) })
                .collect(),
        }
    }
}

/// Convert internal Nodes to proto MerkleProofNode list.
///
/// Note: The proto `MerkleProofNode` doesn't include the `node_hash` field.
/// When converting from `NodeWithHash`, only the node structure is preserved.
/// Clients can recompute the hash if needed using the node data.
fn nodes_to_proto(nodes: katana_rpc_types::trie::Nodes) -> Vec<MerkleProofNode> {
    nodes.0.into_iter().map(|nwh| merkle_node_to_proto(&nwh.node)).collect()
}

fn merkle_node_to_proto(node: &katana_rpc_types::trie::MerkleNode) -> MerkleProofNode {
    match node {
        katana_rpc_types::trie::MerkleNode::Binary { left, right } => MerkleProofNode {
            node: Some(crate::protos::types::merkle_proof_node::Node::Binary(BinaryNode {
                left: Some(ProtoFelt::from(*left)),
                right: Some(ProtoFelt::from(*right)),
            })),
        },
        katana_rpc_types::trie::MerkleNode::Edge { path, length, child } => MerkleProofNode {
            node: Some(crate::protos::types::merkle_proof_node::Node::Edge(EdgeNode {
                path: Some(ProtoFelt::from(*path)),
                length: *length as u32,
                child: Some(ProtoFelt::from(*child)),
            })),
        },
    }
}
