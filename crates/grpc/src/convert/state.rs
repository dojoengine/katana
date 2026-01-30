//! State update type conversions.

use super::to_proto_felt;
use crate::protos::types::{
    DeclaredClass, DeployedContract, Nonce as ProtoNonce, ReplacedClass,
    StateDiff as ProtoStateDiff, StorageDiff, StorageEntry,
};

/// Converts an RPC state diff to proto.
pub fn to_proto_state_diff(diff: &katana_rpc_types::state_update::StateDiff) -> ProtoStateDiff {
    ProtoStateDiff {
        storage_diffs: diff
            .storage_diffs
            .iter()
            .map(|sd| StorageDiff {
                address: Some(to_proto_felt(sd.address.into())),
                storage_entries: sd
                    .storage_entries
                    .iter()
                    .map(|se| StorageEntry {
                        key: Some(to_proto_felt(se.key)),
                        value: Some(to_proto_felt(se.value)),
                    })
                    .collect(),
            })
            .collect(),
        deprecated_declared_classes: diff
            .deprecated_declared_classes
            .iter()
            .map(|c| to_proto_felt(*c))
            .collect(),
        declared_classes: diff
            .declared_classes
            .iter()
            .map(|dc| DeclaredClass {
                class_hash: Some(to_proto_felt(dc.class_hash)),
                compiled_class_hash: Some(to_proto_felt(dc.compiled_class_hash)),
            })
            .collect(),
        deployed_contracts: diff
            .deployed_contracts
            .iter()
            .map(|dc| DeployedContract {
                address: Some(to_proto_felt(dc.address.into())),
                class_hash: Some(to_proto_felt(dc.class_hash)),
            })
            .collect(),
        replaced_classes: diff
            .replaced_classes
            .iter()
            .map(|rc| ReplacedClass {
                contract_address: Some(to_proto_felt(rc.contract_address.into())),
                class_hash: Some(to_proto_felt(rc.class_hash)),
            })
            .collect(),
        nonces: diff
            .nonces
            .iter()
            .map(|n| ProtoNonce {
                contract_address: Some(to_proto_felt(n.contract_address.into())),
                nonce: Some(to_proto_felt(n.nonce)),
            })
            .collect(),
    }
}
