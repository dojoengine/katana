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

use std::ops::Deref;

use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use starknet::core::serde::unsigned_field_element::UfeHex;

/// A wrapper around [`FieldElement`](katana_primitives::FieldElement) that serializes to hex as
/// default.
#[serde_as]
#[derive(Debug, Copy, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FeltAsHex(#[serde_as(serialize_as = "UfeHex")] katana_primitives::Felt);

impl From<katana_primitives::Felt> for FeltAsHex {
    fn from(value: katana_primitives::Felt) -> Self {
        Self(value)
    }
}

impl From<FeltAsHex> for katana_primitives::Felt {
    fn from(value: FeltAsHex) -> Self {
        value.0
    }
}

impl Deref for FeltAsHex {
    type Target = katana_primitives::Felt;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

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
    use rstest::rstest;
    use serde_json::{json, Value};
    use starknet::macros::felt;

    use super::{EstimateFeeSimulationFlag, FeltAsHex, SimulationFlag};

    #[test]
    fn serde_felt() {
        let value = felt!("0x12345");
        let value_as_dec = json!(value);
        let value_as_hex = json!(format!("{value:#x}"));

        let expected_value = FeltAsHex(value);
        let actual_des_value: FeltAsHex = serde_json::from_value(value_as_dec).unwrap();
        assert_eq!(expected_value, actual_des_value, "should deserialize to decimal");

        let actual_ser_value = serde_json::to_value(expected_value).unwrap();
        assert_eq!(value_as_hex, actual_ser_value, "should serialize to hex");
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
