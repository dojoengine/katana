use std::collections::{BTreeMap, BTreeSet};

use katana_primitives::block::BlockHash;
use katana_primitives::class::{ClassHash, CompiledClassHash};
use katana_primitives::contract::{Nonce, StorageKey, StorageValue};
use katana_primitives::{ContractAddress, Felt};
use serde::{Deserialize, Serialize};

use crate::Block;

/// The state update type returns by `/get_state_update` endpoint, with `includeBlock=true`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateUpdateWithBlock {
    pub state_update: StateUpdate,
    pub block: Block,
}

// The main reason why aren't using the state update RPC types is because the state diff
// serialization is different.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum StateUpdate {
    Confirmed(ConfirmedStateUpdate),
    PreConfirmed(PreConfirmedStateUpdate),
}

/// State update of a pre-confirmed block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreConfirmedStateUpdate {
    /// The previous global state root
    pub old_root: Felt,
    /// State diff
    pub state_diff: StateDiff,
}

/// State update of a confirmed block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfirmedStateUpdate {
    /// Block hash
    pub block_hash: BlockHash,
    /// The new global state root
    pub new_root: Felt,
    /// The previous global state root
    pub old_root: Felt,
    /// State diff
    pub state_diff: StateDiff,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageDiff {
    pub key: StorageKey,
    pub value: StorageValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployedContract {
    pub address: ContractAddress,
    pub class_hash: ClassHash,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeclaredContract {
    pub class_hash: ClassHash,
    pub compiled_class_hash: CompiledClassHash,
}

/// State diff returned by the feeder gateway API.
///
/// This type has a different serialization format compared to the JSON-RPC
/// [`StateDiff`](katana_rpc_types::state_update::StateDiff).
///
/// ## Serialization Differences from the RPC Format
///
/// - **`nonces`**
///
///  Serialized as a map object `{ "0x123": "0x1" }`, whereas RPC uses an array `[ {
///  "contract_address": "0x123", "nonce": "0x1" } ]`.
///
/// - **`storage_diffs`**
///
/// Serialized as a map with contract address as key `{ "0x123": [ { "key": "0x1", "value": "0x2" }
/// ] }`, whereas RPC uses an array `[ { "address": "0x123", "storage_entries": [...] } ]`.
///
/// - **`replaced_classes`**
///
/// Uses `"address"` field name, whereas RPC uses `"contract_address"`.
///
/// - **`old_declared_contracts`**
///
/// This field name differs from RPC which uses `"deprecated_declared_classes"`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateDiff {
    pub storage_diffs: BTreeMap<ContractAddress, Vec<StorageDiff>>,
    pub deployed_contracts: Vec<DeployedContract>,
    pub old_declared_contracts: Vec<Felt>,
    pub declared_classes: Vec<DeclaredContract>,
    pub nonces: BTreeMap<ContractAddress, Nonce>,
    pub replaced_classes: Vec<DeployedContract>,
    /// Classes whose compiled class hash (CASM hash) has been recomputed using the Blake2s hash
    /// function.
    ///
    /// Introduced in [SNIP34] (Starknet 0.14.1) as part of the S-Two proving system optimization.
    /// Blake2s hashes are ~3x cheaper to prove compared to Poseidon, significantly reducing the
    /// cost of proving correct Cairo instruction execution within each block.
    ///
    /// [SNIP34]: https://community.starknet.io/t/snip-34-more-efficient-casm-hashes/115979
    ///
    /// ## Migration Mechanism
    ///
    /// Existing classes are migrated gradually - a class hash is migrated the first time SNOS
    /// executes an instruction within a contract using that class. Specifically:
    ///
    /// - **Successful transactions**: All classes used throughout the transaction are migrated.
    /// - **Reverted transactions**: Classes proven before the reversion point are migrated,
    ///   including:
    ///   - The account contract's class (since `validate` must run successfully)
    ///   - Classes where a non-existent entry point invocation was proven
    ///
    /// The compiled class hash updates are applied at the end of each block. Since the execution
    /// environment only references class hashes (never compiled class hashes directly), these
    /// migrations do not affect execution - only the resulting state root.
    #[serde(default)]
    pub migrated_compiled_classes: Vec<DeclaredContract>,
}

impl StateDiff {
    /// Returns a new [`StateDiff`] that contains all updates from `self` and `other`,
    /// preferring the values from `other` when both diffs touch the same entry.
    pub fn merge(mut self, other: StateDiff) -> StateDiff {
        let StateDiff {
            storage_diffs,
            deployed_contracts,
            old_declared_contracts,
            declared_classes,
            nonces,
            replaced_classes,
            migrated_compiled_classes,
        } = other;

        Self::merge_storage_diffs(&mut self.storage_diffs, storage_diffs);
        Self::merge_deployed_contracts(&mut self.deployed_contracts, deployed_contracts);
        Self::merge_deployed_contracts(&mut self.replaced_classes, replaced_classes);
        Self::merge_declared_classes(&mut self.declared_classes, declared_classes);
        Self::merge_declared_classes(
            &mut self.migrated_compiled_classes,
            migrated_compiled_classes,
        );
        Self::merge_old_declared_contracts(
            &mut self.old_declared_contracts,
            old_declared_contracts,
        );
        self.nonces.extend(nonces);

        self
    }

    fn merge_storage_diffs(
        target: &mut BTreeMap<ContractAddress, Vec<StorageDiff>>,
        updates: BTreeMap<ContractAddress, Vec<StorageDiff>>,
    ) {
        for (address, diffs) in updates {
            let entry = target.entry(address).or_default();
            let mut index_by_key: BTreeMap<StorageKey, usize> =
                entry.iter().enumerate().map(|(idx, diff)| (diff.key, idx)).collect();

            for diff in diffs {
                if let Some(idx) = index_by_key.get(&diff.key).copied() {
                    entry[idx] = diff;
                } else {
                    index_by_key.insert(diff.key, entry.len());
                    entry.push(diff);
                }
            }
        }
    }

    fn merge_deployed_contracts(
        target: &mut Vec<DeployedContract>,
        incoming: Vec<DeployedContract>,
    ) {
        let mut index_by_address: BTreeMap<ContractAddress, usize> =
            target.iter().enumerate().map(|(idx, contract)| (contract.address, idx)).collect();

        for contract in incoming {
            if let Some(idx) = index_by_address.get(&contract.address).copied() {
                target[idx] = contract;
            } else {
                index_by_address.insert(contract.address, target.len());
                target.push(contract);
            }
        }
    }

    fn merge_declared_classes(target: &mut Vec<DeclaredContract>, incoming: Vec<DeclaredContract>) {
        let mut index_by_hash: BTreeMap<ClassHash, usize> =
            target.iter().enumerate().map(|(idx, contract)| (contract.class_hash, idx)).collect();

        for declared in incoming {
            if let Some(idx) = index_by_hash.get(&declared.class_hash).copied() {
                target[idx] = declared;
            } else {
                index_by_hash.insert(declared.class_hash, target.len());
                target.push(declared);
            }
        }
    }

    fn merge_old_declared_contracts(target: &mut Vec<Felt>, incoming: Vec<Felt>) {
        let mut seen: BTreeSet<Felt> = target.iter().copied().collect();

        for class_hash in incoming {
            if seen.insert(class_hash) {
                target.push(class_hash);
            }
        }
    }
}

impl<'de> Deserialize<'de> for StateUpdate {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct __Visitor;

        impl<'de> serde::de::Visitor<'de> for __Visitor {
            type Value = StateUpdate;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a state update response")
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut block_hash: Option<BlockHash> = None;
                let mut new_root: Option<Felt> = None;
                let mut old_root: Option<Felt> = None;
                let mut state_diff: Option<StateDiff> = None;

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "block_hash" => {
                            if block_hash.is_some() {
                                return Err(serde::de::Error::duplicate_field("block_hash"));
                            }
                            block_hash = Some(map.next_value()?);
                        }

                        "new_root" => {
                            if new_root.is_some() {
                                return Err(serde::de::Error::duplicate_field("new_root"));
                            }
                            new_root = Some(map.next_value()?);
                        }

                        "old_root" => {
                            if old_root.is_some() {
                                return Err(serde::de::Error::duplicate_field("old_root"));
                            }
                            old_root = Some(map.next_value()?);
                        }

                        "state_diff" => {
                            if state_diff.is_some() {
                                return Err(serde::de::Error::duplicate_field("state_diff"));
                            }
                            state_diff = Some(map.next_value()?);
                        }

                        _ => {
                            let _ = map.next_value::<serde::de::IgnoredAny>()?;
                        }
                    }
                }

                let old_root =
                    old_root.ok_or_else(|| serde::de::Error::missing_field("old_root"))?;
                let state_diff =
                    state_diff.ok_or_else(|| serde::de::Error::missing_field("state_diff"))?;

                // If block_hash and new_root are not present, deserialize as
                // PreConfirmedStateUpdate
                match (block_hash, new_root) {
                    (None, None) => Ok(StateUpdate::PreConfirmed(PreConfirmedStateUpdate {
                        old_root,
                        state_diff,
                    })),

                    (Some(block_hash), Some(new_root)) => {
                        Ok(StateUpdate::Confirmed(ConfirmedStateUpdate {
                            block_hash,
                            new_root,
                            old_root,
                            state_diff,
                        }))
                    }

                    (None, Some(_)) => Err(serde::de::Error::missing_field("block_hash")),
                    (Some(_), None) => Err(serde::de::Error::missing_field("new_root")),
                }
            }
        }

        deserializer.deserialize_map(__Visitor)
    }
}
