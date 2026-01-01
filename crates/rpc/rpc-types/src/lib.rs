#![cfg_attr(not(test), warn(unused_crate_dependencies))]

//! Types used in the Katana JSON-RPC API.

use katana_primitives::block::{BlockHash, BlockNumber};
use katana_primitives::Felt;
use serde::{Deserialize, Serialize};
use serde_utils::{deserialize_u128, deserialize_u64, serialize_as_hex};

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
pub use list::*;
pub use message::*;
pub use outside_execution::*;
pub use receipt::*;
pub use state_update::*;
pub use trace::*;
pub use transaction::*;
pub use trie::*;

/// Block identifier (block hash or number), or tag.
pub type BlockIdOrTag = katana_primitives::block::BlockIdOrTag;
/// Block identifier (block hash or number), or tag that refers only to a confirmed block.
pub type ConfirmedBlockIdOrTag = katana_primitives::block::ConfirmedBlockIdOrTag;

/// Request type for `starknet_call` RPC method.
pub type FunctionCall = katana_primitives::execution::FunctionCall;

/// Finality status of a block or transaction.
pub type FinalityStatus = katana_primitives::block::FinalityStatus;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct CallResponse {
    pub result: Vec<Felt>,
}

/// Fee estimation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct FeeEstimate {
    /// The Ethereum gas consumption of the transaction, charged for L1->L2 messages and, depending
    /// on the block's da_mode, state diffs
    #[serde(serialize_with = "serialize_as_hex", deserialize_with = "deserialize_u64")]
    pub l1_gas_consumed: u64,

    /// The gas price (in wei or fri, depending on the tx version) that was used in the cost
    /// estimation
    #[serde(serialize_with = "serialize_as_hex", deserialize_with = "deserialize_u128")]
    pub l1_gas_price: u128,

    /// The L2 gas consumption of the transaction
    #[serde(serialize_with = "serialize_as_hex", deserialize_with = "deserialize_u64")]
    pub l2_gas_consumed: u64,

    /// The L2 gas price (in wei or fri, depending on the tx version) that was used in the cost
    /// estimation
    #[serde(serialize_with = "serialize_as_hex", deserialize_with = "deserialize_u128")]
    pub l2_gas_price: u128,

    /// The Ethereum data gas consumption of the transaction
    #[serde(serialize_with = "serialize_as_hex", deserialize_with = "deserialize_u64")]
    pub l1_data_gas_consumed: u64,

    /// The data gas price (in wei or fri, depending on the tx version) that was used in the cost
    /// estimation
    #[serde(serialize_with = "serialize_as_hex", deserialize_with = "deserialize_u128")]
    pub l1_data_gas_price: u128,

    /// The estimated fee for the transaction (in wei or fri, depending on the tx version), equals
    /// to l1_gas_consumed*l1_gas_price + l1_data_gas_consumed*l1_data_gas_price +
    /// l2_gas_consumed*l2_gas_price
    #[serde(serialize_with = "serialize_as_hex", deserialize_with = "deserialize_u128")]
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
        use serde::de::{self, MapAccess, Visitor};

        struct SyncingResponseVisitor;

        impl<'de> Visitor<'de> for SyncingResponseVisitor {
            type Value = SyncingResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(
                    "either `false` or an object with fields: starting_block_hash, \
                     starting_block_num, current_block_hash, current_block_num, \
                     highest_block_hash, highest_block_num",
                )
            }

            fn visit_bool<E: de::Error>(self, value: bool) -> Result<Self::Value, E> {
                if !value {
                    Ok(SyncingResponse::NotSyncing)
                } else {
                    Err(E::custom("expected `false` for not syncing state"))
                }
            }

            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
                let status = SyncStatus::deserialize(de::value::MapAccessDeserializer::new(map))?;
                Ok(SyncingResponse::Syncing(status))
            }
        }

        deserializer.deserialize_any(SyncingResponseVisitor)
    }
}

#[cfg(test)]
mod tests {
    use katana_primitives::{felt, Felt};
    use rstest::rstest;
    use serde_json::{json, Value};

    use super::{
        BlockIdOrTag, ConfirmedBlockIdOrTag, EstimateFeeSimulationFlag, FeeEstimate,
        FinalityStatus, FunctionCall, SimulationFlag, SyncStatus, SyncingResponse,
    };

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
    #[case(FinalityStatus::AcceptedOnL2, json!("ACCEPTED_ON_L2"))]
    #[case(FinalityStatus::AcceptedOnL1, json!("ACCEPTED_ON_L1"))]
    #[case(FinalityStatus::PreConfirmed, json!("PRE_CONFIRMED"))]
    fn finality_status_serde(#[case] status: FinalityStatus, #[case] json: Value) {
        let serialized = serde_json::to_value(&status).unwrap();
        assert_eq!(serialized, json);
        let deserialized: FinalityStatus = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized, status);
    }

    #[rstest]
    #[case(json!("INVALID_STATUS"))]
    #[case(json!("accepted_on_l2"))]
    #[case(json!("accepted_on_l1"))]
    #[case(json!("pre_confirmed"))]
    #[case(json!(""))]
    #[case(json!(123))]
    #[case(json!(null))]
    #[case(json!(false))]
    fn invalid_finality_status(#[case] invalid: Value) {
        let result = serde_json::from_value::<FinalityStatus>(invalid.clone());
        assert!(result.is_err(), "expected error for invalid value: {invalid}");
    }

    #[rstest]
    #[case(BlockIdOrTag::Latest, json!("latest"))]
    #[case(BlockIdOrTag::L1Accepted, json!("l1_accepted"))]
    #[case(BlockIdOrTag::PreConfirmed, json!("pre_confirmed"))]
    #[case(BlockIdOrTag::Hash(felt!("0x123")), json!({"block_hash": "0x123"}))]
    #[case(BlockIdOrTag::Number(42), json!({"block_number": 42}))]
    fn block_id_or_tag_serde(#[case] block_id: BlockIdOrTag, #[case] json: Value) {
        let serialized = serde_json::to_value(&block_id).unwrap();
        assert_eq!(serialized, json);
        let deserialized: BlockIdOrTag = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized, block_id);
    }

    #[rstest]
    #[case(json!("invalid_tag"))]
    #[case(json!("LATEST"))]
    #[case(json!("L1_ACCEPTED"))]
    #[case(json!("PRE_CONFIRMED"))]
    #[case(json!({"block_hash": "0x123", "block_number": 42}))]
    #[case(json!({}))]
    #[case(json!({"invalid_field": "value"}))]
    #[case(json!(123))]
    #[case(json!(null))]
    #[case(json!(true))]
    #[case(json!([]))]
    fn invalid_block_id_or_tag(#[case] invalid: Value) {
        let result = serde_json::from_value::<BlockIdOrTag>(invalid.clone());
        assert!(result.is_err(), "expected error for invalid value: {invalid}");
    }

    #[rstest]
    #[case(ConfirmedBlockIdOrTag::Latest, json!("latest"))]
    #[case(ConfirmedBlockIdOrTag::L1Accepted, json!("l1_accepted"))]
    #[case(ConfirmedBlockIdOrTag::Hash(felt!("0x456")), json!({"block_hash": "0x456"}))]
    #[case(ConfirmedBlockIdOrTag::Number(100), json!({"block_number": 100}))]
    fn confirmed_block_id_or_tag_serde(
        #[case] block_id: ConfirmedBlockIdOrTag,
        #[case] json: Value,
    ) {
        let serialized = serde_json::to_value(&block_id).unwrap();
        assert_eq!(serialized, json);
        let deserialized: ConfirmedBlockIdOrTag = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized, block_id);
    }

    #[rstest]
    #[case(json!("pre_confirmed"))]
    #[case(json!("invalid_tag"))]
    #[case(json!("LATEST"))]
    #[case(json!("L1_ACCEPTED"))]
    #[case(json!({"block_hash": "0x123", "block_number": 42}))]
    #[case(json!({}))]
    #[case(json!({"invalid_field": "value"}))]
    #[case(json!(123))]
    #[case(json!(null))]
    #[case(json!(false))]
    #[case(json!([]))]
    fn invalid_confirmed_block_id_or_tag(#[case] invalid: Value) {
        let result = serde_json::from_value::<ConfirmedBlockIdOrTag>(invalid.clone());
        assert!(result.is_err(), "expected error for invalid value: {invalid}");
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

    #[test]
    fn function_call_serde() {
        let function_call = FunctionCall {
            entry_point_selector: felt!("0x100"),
            contract_address: felt!("0x200").into(),
            calldata: vec![felt!("0x1"), felt!("0x2"), felt!("0x3")],
        };

        let json = json!({
            "entry_point_selector": "0x100",
            "contract_address": "0x200",
            "calldata": ["0x1", "0x2", "0x3"]
        });

        let serialized = serde_json::to_value(&function_call).unwrap();
        assert_eq!(serialized, json);

        let deserialized = serde_json::from_value::<FunctionCall>(json).unwrap();
        assert_eq!(deserialized, function_call);
    }

    #[test]
    fn fee_estimate_serde() {
        let fee_estimate = FeeEstimate {
            l1_gas_consumed: 100,
            l1_gas_price: 1000,
            l2_gas_consumed: 200,
            l2_gas_price: 2000,
            l1_data_gas_consumed: 300,
            l1_data_gas_price: 3000,
            overall_fee: 1400000,
        };

        let json = json!({
            "l1_gas_consumed": "0x64",
            "l1_gas_price": "0x3e8",
            "l2_gas_consumed": "0xc8",
            "l2_gas_price": "0x7d0",
            "l1_data_gas_consumed": "0x12c",
            "l1_data_gas_price": "0xbb8",
            "overall_fee": "0x155cc0"
        });

        let serialized = serde_json::to_value(&fee_estimate).unwrap();
        assert_eq!(serialized, json);

        let deserialized = serde_json::from_value::<FeeEstimate>(json).unwrap();
        assert_eq!(deserialized, fee_estimate);
    }

    #[test]
    fn syncing_response_not_syncing_serde() {
        let response = SyncingResponse::NotSyncing;

        let expected_json = json!(false);

        // Serialization
        let serialized = serde_json::to_value(&response).unwrap();
        assert_eq!(serialized, expected_json);

        // Deserialization
        let deserialized: SyncingResponse = serde_json::from_value(expected_json).unwrap();
        assert_eq!(deserialized, SyncingResponse::NotSyncing);
    }

    #[test]
    fn syncing_response_syncing_serde() {
        let sync_status = SyncStatus {
            starting_block_hash: felt!("0x1"),
            starting_block_num: 100,
            current_block_hash: felt!("0x2"),
            current_block_num: 200,
            highest_block_hash: felt!("0x3"),
            highest_block_num: 300,
        };

        let response = SyncingResponse::Syncing(sync_status.clone());

        let expected_json = json!({
            "starting_block_hash": "0x1",
            "starting_block_num": 100,
            "current_block_hash": "0x2",
            "current_block_num": 200,
            "highest_block_hash": "0x3",
            "highest_block_num": 300,
        });

        // Serialization
        let serialized = serde_json::to_value(&response).unwrap();
        assert_eq!(serialized, expected_json);

        // Deserialization
        let deserialized: SyncingResponse = serde_json::from_value(expected_json).unwrap();
        assert_eq!(deserialized, SyncingResponse::Syncing(sync_status));
    }

    #[rstest]
    #[case(json!(true))]
    #[case(json!("invalid"))]
    #[case(json!(123))]
    #[case(json!(null))]
    #[case(json!([]))]
    #[case(json!({"invalid_field": "value"}))]
    #[case(json!({"starting_block_hash": "0x1"}))] // Missing required fields
    fn invalid_syncing_response(#[case] invalid: Value) {
        let result = serde_json::from_value::<SyncingResponse>(invalid.clone());
        assert!(result.is_err(), "expected error for invalid value: {invalid}");
    }
}
