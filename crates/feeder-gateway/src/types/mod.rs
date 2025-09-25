//! # Feeder Gateway Types
//!
//! This module defines types that mirror the data structures returned by the Starknet feeder
//! gateway API. Ideally, we would not need to redefine these types, but the feeder gateway requires
//! its own type definitions due to fundamental serialization incompatibilities with the existing
//! types in `katana-primitives` and `katana-rpc-types`. For objects that share the same format, we
//! reuse existing RPC or primitive types whenever possible.
//!
//! ## Affected Types
//!
//! - [`DataAvailabilityMode`]: Integer-based representation
//! - [`ResourceBounds`]: Custom numeric handling
//! - [`ResourceBoundsMapping`]: Uppercase field names, optional `L1_DATA_GAS`
//! - [`InvokeTxV3`]: Uses the custom DA mode and resource bounds
//! - [`DeclareTxV3`]: Uses the custom DA mode and resource bounds
//! - [`DeployAccountTxV1`]: Optional `contract_address` field
//! - [`DeployAccountTxV3`]: Uses the custom DA mode and resource bounds
//! - [`L1HandlerTx`]: Optional `nonce` field

use std::collections::BTreeMap;

use katana_primitives::block::{BlockHash, BlockNumber};
pub use katana_primitives::class::CasmContractClass;
use katana_primitives::class::{ClassHash, CompiledClassHash};
use katana_primitives::contract::{Nonce, StorageKey, StorageValue};
use katana_primitives::da::L1DataAvailabilityMode;
use katana_primitives::{ContractAddress, Felt};
pub use katana_rpc_types::class::SierraClass;
use serde::{Deserialize, Serialize};
use starknet::core::types::ResourcePrice;

mod conversion;
mod error;
mod receipt;
mod transaction;

pub use error::*;
pub use receipt::*;
pub use transaction::*;

/// The contract class type returns by `/get_class_by_hash` endpoint.
pub type ContractClass = katana_rpc_types::Class;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum BlockId {
    Number(BlockNumber),
    Hash(BlockHash),
    Latest,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum BlockStatus {
    #[serde(rename = "PENDING")]
    Pending,

    #[serde(rename = "ABORTED")]
    Aborted,

    #[serde(rename = "REVERTED")]
    Reverted,

    #[serde(rename = "ACCEPTED_ON_L2")]
    AcceptedOnL2,

    #[serde(rename = "ACCEPTED_ON_L1")]
    AcceptedOnL1,
}

// The reason why we're not using the GasPrices from the `katana_primitives` crate is because
// the serde impl is different. So for now, lets just use starknet-rs types. The type isn't
// that complex anyway so the conversion is simple. But if we can use the primitive types, we
// should.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub status: BlockStatus,
    #[serde(default)]
    pub block_hash: Option<BlockHash>,
    #[serde(default)]
    pub block_number: Option<BlockNumber>,
    pub parent_block_hash: BlockHash,
    pub l1_da_mode: L1DataAvailabilityMode,
    pub timestamp: u64,
    pub sequencer_address: Option<ContractAddress>,
    #[serde(default)]
    pub starknet_version: Option<String>,
    #[serde(default = "default_l2_gas_price")]
    pub l2_gas_price: ResourcePrice,
    pub l1_gas_price: ResourcePrice,
    pub l1_data_gas_price: ResourcePrice,
    #[serde(default)]
    pub event_commitment: Option<Felt>,
    #[serde(default)]
    pub state_diff_commitment: Option<Felt>,
    #[serde(default)]
    pub state_root: Option<Felt>,
    #[serde(default)]
    pub transaction_commitment: Option<Felt>,
    pub transaction_receipts: Vec<ConfirmedReceipt>,
    pub transactions: Vec<ConfirmedTransaction>,
}

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

// todo(kariy): merge the serialization of gateway into the rpc types
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateDiff {
    pub storage_diffs: BTreeMap<ContractAddress, Vec<StorageDiff>>,
    pub deployed_contracts: Vec<DeployedContract>,
    pub old_declared_contracts: Vec<Felt>,
    pub declared_classes: Vec<DeclaredContract>,
    pub nonces: BTreeMap<ContractAddress, Nonce>,
    pub replaced_classes: Vec<DeployedContract>,
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

fn default_l2_gas_price() -> ResourcePrice {
    ResourcePrice { price_in_fri: Felt::from(1), price_in_wei: Felt::from(1) }
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
