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
pub mod message;
pub mod outside_execution;
pub mod receipt;
pub mod state_update;
pub mod trace;
pub mod transaction;
pub mod trie;

use serde::{Deserialize, Serialize};

pub type FunctionCall = starknet::core::types::FunctionCall;

pub type FeeEstimate = starknet::core::types::FeeEstimate;

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

pub type SyncingStatus = starknet::core::types::SyncStatusType;

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
