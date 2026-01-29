use cainome::cairo_serde::{deserialize_from_hex, serialize_as_hex};
use cainome::cairo_serde_derive::CairoSerde;
use cainome_cairo_serde::CairoSerde;
use serde::{Deserialize, Serialize};
use starknet::macros::selector;
use starknet_crypto::Felt;

/// A single call to be executed as part of an outside execution.
#[derive(Clone, CairoSerde, Serialize, Deserialize, PartialEq, Debug)]
pub struct Call {
    /// Contract address to call.
    pub to: Felt,
    /// Function selector to invoke.
    pub selector: Felt,
    /// Arguments to pass to the function.
    pub calldata: Vec<Felt>,
}

impl From<Call> for starknet::core::types::Call {
    fn from(val: Call) -> Self {
        starknet::core::types::Call {
            to: val.to,
            selector: val.selector,
            calldata: val.calldata,
        }
    }
}
impl From<starknet::core::types::Call> for Call {
    fn from(val: starknet::core::types::Call) -> Self {
        Call {
            to: val.to,
            selector: val.selector,
            calldata: val.calldata,
        }
    }
}

/// Nonce channel
#[derive(Clone, CairoSerde, PartialEq, Debug, Serialize, Deserialize)]
pub struct NonceChannel(
    pub Felt,
    #[serde(
        serialize_with = "serialize_as_hex",
        deserialize_with = "deserialize_from_hex"
    )]
    pub u128,
);

/// Outside execution version 2 (SNIP-9 standard).
#[derive(Clone, CairoSerde, Serialize, Deserialize, PartialEq, Debug)]
pub struct OutsideExecutionV2 {
    /// Address allowed to initiate execution ('ANY_CALLER' for unrestricted).
    pub caller: Felt,
    /// Unique nonce to prevent signature reuse.
    pub nonce: Felt,
    /// Timestamp after which execution is valid.
    #[serde(
        serialize_with = "serialize_as_hex",
        deserialize_with = "deserialize_from_hex"
    )]
    pub execute_after: u64,
    /// Timestamp before which execution is valid.
    #[serde(
        serialize_with = "serialize_as_hex",
        deserialize_with = "deserialize_from_hex"
    )]
    pub execute_before: u64,
    /// Calls to execute in order.
    pub calls: Vec<Call>,
}

/// Non-standard extension of the [`OutsideExecutionV2`] supported by the Cartridge Controller.
#[derive(Clone, CairoSerde, Serialize, Deserialize, PartialEq, Debug)]
pub struct OutsideExecutionV3 {
    /// Address allowed to initiate execution ('ANY_CALLER' for unrestricted).
    pub caller: Felt,
    /// Nonce.
    pub nonce: NonceChannel,
    /// Timestamp after which execution is valid.
    #[serde(
        serialize_with = "serialize_as_hex",
        deserialize_with = "deserialize_from_hex"
    )]
    pub execute_after: u64,
    /// Timestamp before which execution is valid.
    #[serde(
        serialize_with = "serialize_as_hex",
        deserialize_with = "deserialize_from_hex"
    )]
    pub execute_before: u64,
    /// Calls to execute in order.
    pub calls: Vec<Call>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
// #[serde(untagged)]
pub enum OutsideExecution {
    /// SNIP-9 standard version.
    V2(OutsideExecutionV2),
    /// Cartridge/Controller extended version.
    V3(OutsideExecutionV3),
}

impl OutsideExecution {
    pub fn calls(self: &OutsideExecution) -> Vec<Call> {
        match self {
            OutsideExecution::V2(outside_execution_v2) => outside_execution_v2.calls.clone(),
            OutsideExecution::V3(outside_execution_v3) => outside_execution_v3.calls.clone(),
        }
    }
    pub fn selector(self: &OutsideExecution) -> Felt {
        match self {
            OutsideExecution::V2(_) => selector!("execute_from_outside_v2"),
            OutsideExecution::V3(_) => selector!("execute_from_outside_v3"),
        }
    }
    pub fn caller(self: &OutsideExecution) -> Felt {
        match self {
            OutsideExecution::V2(outside_execution_v2) => outside_execution_v2.caller,
            OutsideExecution::V3(outside_execution_v3) => outside_execution_v3.caller,
        }
    }
    pub fn nonce(self: &OutsideExecution) -> Felt {
        match self {
            OutsideExecution::V2(outside_execution_v2) => outside_execution_v2.nonce,
            OutsideExecution::V3(_) => {
                unreachable!()
            }
        }
    }
    pub fn execute_after(self: &OutsideExecution) -> u64 {
        match self {
            OutsideExecution::V2(outside_execution_v2) => outside_execution_v2.execute_after,
            OutsideExecution::V3(outside_execution_v3) => outside_execution_v3.execute_after,
        }
    }
    pub fn execute_before(self: &OutsideExecution) -> u64 {
        match self {
            OutsideExecution::V2(outside_execution_v2) => outside_execution_v2.execute_before,
            OutsideExecution::V3(outside_execution_v3) => outside_execution_v3.execute_before,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct SignedOutsideExecution {
    pub address: Felt,
    pub outside_execution: OutsideExecution,
    pub signature: Vec<Felt>,
}

impl SignedOutsideExecution {
    pub fn build_execute_from_outside_call(self: &SignedOutsideExecution) -> Call {
        let outside_execution = self.outside_execution.clone();

        let mut calldata = match outside_execution.clone() {
            OutsideExecution::V2(outside_execution_v2) => {
                OutsideExecutionV2::cairo_serialize(&outside_execution_v2)
            }
            OutsideExecution::V3(outside_execution_v3) => {
                OutsideExecutionV3::cairo_serialize(&outside_execution_v3)
            }
        };

        calldata.push(self.signature.len().into());
        calldata.extend(self.signature.clone());

        Call {
            to: self.address,
            selector: outside_execution.selector(),
            calldata,
        }
    }
}
