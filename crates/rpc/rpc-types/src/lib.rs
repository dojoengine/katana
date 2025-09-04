#![cfg_attr(not(test), warn(unused_crate_dependencies))]

//! Types used in the Katana JSON-RPC API.
//!
//! Most of the types defined in this crate are simple wrappers around types imported from
//! `starknet-rs`.

pub mod account;
pub mod block;
pub mod broadcasted;
pub mod class;
pub mod event;
pub mod list;
pub mod message;
pub mod outside_execution;
pub mod receipt;
pub mod state_update;
pub mod trace;
pub mod transaction;
pub mod trie;

pub use block::*;
pub use broadcasted::*;
pub use class::*;
pub use event::*;
use katana_primitives::block::{BlockHash, BlockNumber};
pub use message::*;
pub use outside_execution::*;
pub use receipt::*;
use serde::{Deserialize, Serialize};
pub use transaction::*;
pub use trie::*;

/// Request type for `starknet_call` RPC method.
pub type FunctionCall = katana_primitives::execution::FunctionCall;

pub type FeeEstimate = starknet::core::types::FeeEstimate;

pub type MessageFeeEstimate = starknet::core::types::MessageFeeEstimate;

/// Simulation flags for `starknet_estimateFee` RPC method.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum EstimateFeeSimulationFlag {
    #[serde(rename = "SKIP_VALIDATE")]
    SkipValidate,
}

/// Simulation flags for `starknet_simulationTransactions` RPC method.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SimulationFlag {
    #[serde(rename = "SKIP_VALIDATE")]
    SkipValidate,
    #[serde(rename = "SKIP_FEE_CHARGE")]
    SkipFeeCharge,
}

/// A Starknet client node's synchronization status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncingResponse {
    /// The node is synchronizing.
    Syncing(SyncStatus),
    /// The node is not synchronizing.
    NotSyncing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncStatus {
    /// The hash of the block from which the sync started
    pub starting_block_hash: BlockHash,
    /// The number (height) of the block from which the sync started
    pub starting_block_num: BlockNumber,
    /// The hash of the current block being synchronized
    pub current_block_hash: BlockHash,
    /// The number (height) of the current block being synchronized
    pub current_block_num: BlockNumber,
    /// The hash of the estimated highest block to be synchronized
    pub highest_block_hash: BlockHash,
    /// The number (height) of the estimated highest block to be synchronized
    pub highest_block_num: BlockNumber,
}

impl Serialize for SyncingResponse {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::NotSyncing => serializer.serialize_bool(false),
            Self::Syncing(sync_status) => SyncStatus::serialize(sync_status, serializer),
        }
    }
}

impl<'de> Deserialize<'de> for SyncingResponse {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Helper {
            Boolean(bool),
            SyncStatus(SyncStatus),
        }

        match Helper::deserialize(deserializer)? {
            Helper::SyncStatus(value) => Ok(Self::Syncing(value)),
            Helper::Boolean(value) => match value {
                true => Err(serde::de::Error::custom("invalid boolean value")),
                false => Ok(Self::NotSyncing),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use katana_primitives::{felt, Felt};
    use rstest::rstest;
    use serde_json::{json, Value};

    use super::{EstimateFeeSimulationFlag, SimulationFlag};

    #[rstest::rstest]
    #[case(felt!("0x0"), json!("0x0"))]
    #[case(felt!("0xa"), json!("0xa"))]
    #[case(felt!("0x1337"), json!("0x1337"))]
    #[case(felt!("0x12345"), json!("0x12345"))]
    fn felt_serde_hex(#[case] felt: Felt, #[case] json: Value) {
        let serialized = serde_json::to_value(&felt).unwrap();
        assert_eq!(serialized, json);
        let deserialized: Felt = serde_json::from_value(serialized).unwrap();
        assert_eq!(deserialized, felt);
    }

    #[rstest]
    #[case(EstimateFeeSimulationFlag::SkipValidate, json!("SKIP_VALIDATE"))]
    fn estimate_fee_simulation_flags_serde(
        #[case] flag: EstimateFeeSimulationFlag,
        #[case] json: Value,
    ) {
        let serialized = serde_json::to_value(&flag).unwrap();
        assert_eq!(serialized, json);
        let deserialized: EstimateFeeSimulationFlag = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized, flag);
    }

    #[rstest]
    #[case(json!("INVALID_FLAG"))]
    #[case(json!("skip_validate"))]
    #[case(json!(""))]
    #[case(json!(123))]
    #[case(json!(null))]
    #[case(json!(true))]
    fn invalid_estimate_fee_simulation_flags(#[case] invalid: Value) {
        let result = serde_json::from_value::<EstimateFeeSimulationFlag>(invalid.clone());
        assert!(result.is_err(), "expected error for invalid value: {invalid}");
    }

    #[rstest]
    #[case(SimulationFlag::SkipValidate, json!("SKIP_VALIDATE"))]
    #[case(SimulationFlag::SkipFeeCharge, json!("SKIP_FEE_CHARGE"))]
    fn simulation_flags_serde(#[case] flag: SimulationFlag, #[case] json: Value) {
        let serialized = serde_json::to_value(&flag).unwrap();
        assert_eq!(serialized, json);
        let deserialized: SimulationFlag = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized, flag);
    }

    #[rstest]
    #[case(json!("INVALID_FLAG"))]
    #[case(json!("skip_validate"))]
    #[case(json!("skip_fee_charge"))]
    #[case(json!(""))]
    #[case(json!(123))]
    #[case(json!(null))]
    #[case(json!(false))]
    fn invalid_simulation_flags(#[case] invalid: Value) {
        let result = serde_json::from_value::<SimulationFlag>(invalid.clone());
        assert!(result.is_err(), "expected error for invalid value: {invalid}");
    }
}
