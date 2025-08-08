use katana_primitives::class::ClassHash;
use serde::{Deserialize, Serialize};
use starknet::core::types::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, NonceUpdate, StorageEntry,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MaybePreConfirmedStateUpdate {
    PreConfirmed(PreConfirmedStateUpdate),
    Update(StateUpdate),
}

impl From<starknet::core::types::MaybePreConfirmedStateUpdate> for MaybePreConfirmedStateUpdate {
    fn from(value: starknet::core::types::MaybePreConfirmedStateUpdate) -> Self {
        match value {
            starknet::core::types::MaybePreConfirmedStateUpdate::PreConfirmedUpdate(pending) => {
                MaybePreConfirmedStateUpdate::PreConfirmed(pending.into())
            }
            starknet::core::types::MaybePreConfirmedStateUpdate::Update(update) => {
                MaybePreConfirmedStateUpdate::Update(update.into())
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StateUpdate(starknet::core::types::StateUpdate);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PreConfirmedStateUpdate(starknet::core::types::PreConfirmedStateUpdate);

#[derive(Debug, Clone, Serialize)]
#[serde(transparent)]
pub struct StateDiff(pub starknet::core::types::StateDiff);

impl From<starknet::core::types::StateUpdate> for StateUpdate {
    fn from(value: starknet::core::types::StateUpdate) -> Self {
        Self(value)
    }
}

impl From<katana_primitives::state::StateUpdates> for StateDiff {
    fn from(value: katana_primitives::state::StateUpdates) -> Self {
        let nonces: Vec<NonceUpdate> = value
            .nonce_updates
            .into_iter()
            .map(|(addr, nonce)| NonceUpdate { nonce, contract_address: addr.into() })
            .collect();

        let deprecated_declared_classes: Vec<ClassHash> =
            value.deprecated_declared_classes.into_iter().collect();

        let declared_classes: Vec<DeclaredClassItem> = value
            .declared_classes
            .into_iter()
            .map(|(class_hash, compiled_class_hash)| DeclaredClassItem {
                class_hash,
                compiled_class_hash,
            })
            .collect();

        let deployed_contracts: Vec<DeployedContractItem> = value
            .deployed_contracts
            .into_iter()
            .map(|(addr, class_hash)| DeployedContractItem { address: addr.into(), class_hash })
            .collect();

        let storage_diffs: Vec<ContractStorageDiffItem> = value
            .storage_updates
            .into_iter()
            .map(|(addr, entries)| ContractStorageDiffItem {
                address: addr.into(),
                storage_entries: entries
                    .into_iter()
                    .map(|(key, value)| StorageEntry { key, value })
                    .collect(),
            })
            .collect();

        Self(starknet::core::types::StateDiff {
            nonces,
            storage_diffs,
            declared_classes,
            deployed_contracts,
            deprecated_declared_classes,
            replaced_classes: Default::default(),
        })
    }
}

impl From<starknet::core::types::PreConfirmedStateUpdate> for PreConfirmedStateUpdate {
    fn from(value: starknet::core::types::PreConfirmedStateUpdate) -> Self {
        Self(value)
    }
}
