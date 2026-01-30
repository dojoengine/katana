//! State update type conversions.

use katana_primitives::Felt;

use crate::protos::types::{
    DeclaredClass, DeployedContract, Felt as ProtoFelt, Nonce as ProtoNonce, ReplacedClass,
    StateDiff as ProtoStateDiff, StorageDiff, StorageEntry,
};

/// Convert RPC state diff to proto.
impl From<&katana_rpc_types::state_update::StateDiff> for ProtoStateDiff {
    fn from(diff: &katana_rpc_types::state_update::StateDiff) -> Self {
        ProtoStateDiff {
            storage_diffs: diff
                .storage_diffs
                .iter()
                .map(|sd| StorageDiff {
                    address: Some(ProtoFelt::from(Felt::from(sd.address))),
                    storage_entries: sd
                        .storage_entries
                        .iter()
                        .map(|se| StorageEntry {
                            key: Some(se.key.into()),
                            value: Some(se.value.into()),
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
                .map(|dc| DeclaredClass {
                    class_hash: Some(dc.class_hash.into()),
                    compiled_class_hash: Some(dc.compiled_class_hash.into()),
                })
                .collect(),
            deployed_contracts: diff
                .deployed_contracts
                .iter()
                .map(|dc| DeployedContract {
                    address: Some(ProtoFelt::from(Felt::from(dc.address))),
                    class_hash: Some(dc.class_hash.into()),
                })
                .collect(),
            replaced_classes: diff
                .replaced_classes
                .iter()
                .map(|rc| ReplacedClass {
                    contract_address: Some(ProtoFelt::from(Felt::from(rc.contract_address))),
                    class_hash: Some(rc.class_hash.into()),
                })
                .collect(),
            nonces: diff
                .nonces
                .iter()
                .map(|n| ProtoNonce {
                    contract_address: Some(ProtoFelt::from(Felt::from(n.contract_address))),
                    nonce: Some(n.nonce.into()),
                })
                .collect(),
        }
    }
}
