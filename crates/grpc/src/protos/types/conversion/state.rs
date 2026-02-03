//! State update type conversions.

use katana_primitives::Felt;

use crate::protos::common::Felt as ProtoFelt;
use crate::protos::types::{
    DeclaredClass, DeployedContract, Nonce as ProtoNonce, ReplacedClass,
    StateDiff as ProtoStateDiff, StorageDiff, StorageEntry,
};

/// Convert RPC state diff to proto.
impl From<&katana_rpc_types::state_update::StateDiff> for ProtoStateDiff {
    fn from(diff: &katana_rpc_types::state_update::StateDiff) -> Self {
        ProtoStateDiff {
            storage_diffs: diff
                .storage_diffs
                .iter()
                .map(|(address, entries)| StorageDiff {
                    address: Some(ProtoFelt::from(Felt::from(*address))),
                    storage_entries: entries
                        .iter()
                        .map(|(key, value)| StorageEntry {
                            key: Some((*key).into()),
                            value: Some((*value).into()),
                        })
                        .collect(),
                })
                .collect(),
            deprecated_declared_classes: diff
                .deprecated_declared_classes
                .iter()
                .map(|c| ProtoFelt::from(*c))
                .collect(),
            declared_classes: diff
                .declared_classes
                .iter()
                .map(|(class_hash, compiled_class_hash)| DeclaredClass {
                    class_hash: Some((*class_hash).into()),
                    compiled_class_hash: Some((*compiled_class_hash).into()),
                })
                .collect(),
            deployed_contracts: diff
                .deployed_contracts
                .iter()
                .map(|(address, class_hash)| DeployedContract {
                    address: Some(ProtoFelt::from(Felt::from(*address))),
                    class_hash: Some((*class_hash).into()),
                })
                .collect(),
            replaced_classes: diff
                .replaced_classes
                .iter()
                .map(|(contract_address, class_hash)| ReplacedClass {
                    contract_address: Some(ProtoFelt::from(Felt::from(*contract_address))),
                    class_hash: Some((*class_hash).into()),
                })
                .collect(),
            nonces: diff
                .nonces
                .iter()
                .map(|(contract_address, nonce)| ProtoNonce {
                    contract_address: Some(ProtoFelt::from(Felt::from(*contract_address))),
                    nonce: Some((*nonce).into()),
                })
                .collect(),
        }
    }
}
