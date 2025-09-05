#![cfg_attr(not(test), warn(unused_crate_dependencies))]

//! Types used in the Katana JSON-RPC API.

use katana_primitives::block::{BlockHash, BlockNumber};
use katana_primitives::Felt;
use serde::{Deserialize, Serialize};

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
pub use message::*;
pub use outside_execution::*;
pub use receipt::*;
pub use transaction::*;
pub use trie::*;

/// Request type for `starknet_call` RPC method.
pub type FunctionCall = katana_primitives::execution::FunctionCall;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct CallResponse {
    pub result: Vec<Felt>,
}

/// Message fee estimation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct FeeEstimate {
    /// The Ethereum gas consumption of the transaction, charged for L1->L2 messages and, depending
    /// on the block's da_mode, state diffs
    pub l1_gas_consumed: u64,
    /// The gas price (in wei or fri, depending on the tx version) that was used in the cost
    /// estimation
    pub l1_gas_price: u128,
    /// The L2 gas consumption of the transaction
    pub l2_gas_consumed: u64,
    /// The L2 gas price (in wei or fri, depending on the tx version) that was used in the cost
    /// estimation
    pub l2_gas_price: u128,
    /// The Ethereum data gas consumption of the transaction
    pub l1_data_gas_consumed: u64,
    /// The data gas price (in wei or fri, depending on the tx version) that was used in the cost
    /// estimation
    pub l1_data_gas_price: u128,
    /// The estimated fee for the transaction (in wei or fri, depending on the tx version), equals
    /// to l1_gas_consumed*l1_gas_price + l1_data_gas_consumed*l1_data_gas_price +
    /// l2_gas_consumed*l2_gas_price
    pub overall_fee: u128,
}

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
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::NotSyncing => serializer.serialize_bool(false),
            Self::Syncing(sync_status) => SyncStatus::serialize(sync_status, serializer),
        }
    }
}

impl<'de> Deserialize<'de> for SyncingResponse {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::Deserialize;
        use serde::__private::de::{Content, ContentRefDeserializer};

        let content = <Content as Deserialize>::deserialize(deserializer)?;
        let deserializer = ContentRefDeserializer::<D::Error>::new(&content);

        if let Ok(bool) = <bool as Deserialize>::deserialize(deserializer) {
            // The only valid boolean value is `false` which indicates that the node is not syncing.
            if bool == false {
                return Ok(Self::NotSyncing);
            };
        } else if let Ok(value) = <SyncStatus as Deserialize>::deserialize(deserializer) {
            return Ok(Self::Syncing(value));
        }

        Err(serde::de::Error::custom(
            "expected either `false` or an object with fields: starting_block_hash, \
             starting_block_num, current_block_hash, current_block_num, highest_block_hash, \
             highest_block_num",
        ))
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
