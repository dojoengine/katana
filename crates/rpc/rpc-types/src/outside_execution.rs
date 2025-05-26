//! Outside execution types for Starknet accounts.
//!
//! Outside execution (meta-transactions) allows protocols to submit transactions
//! on behalf of user accounts with their signatures. This enables delayed orders,
//! fee subsidy, and other advanced transaction patterns.
//!
//! Based on [SNIP-9](https://github.com/starknet-io/SNIPs/blob/main/SNIPS/snip-9.md).

use cainome::cairo_serde_derive::CairoSerde;
use katana_primitives::{ContractAddress, Felt};
use serde::{Deserialize, Serialize};

/// A single call to be executed as part of an outside execution.
#[derive(Clone, CairoSerde, Serialize, Deserialize, PartialEq, Debug)]
pub struct Call {
    /// Contract address to call.
    pub to: ContractAddress,
    /// Function selector to invoke.
    pub selector: Felt,
    /// Arguments to pass to the function.
    pub calldata: Vec<Felt>,
}

/// Outside execution version 2 (SNIP-9 standard).
#[derive(Clone, CairoSerde, Serialize, Deserialize, PartialEq, Debug)]
pub struct OutsideExecutionV2 {
    /// Address allowed to initiate execution ('ANY_CALLER' for unrestricted).
    pub caller: ContractAddress,
    /// Unique nonce to prevent signature reuse.
    pub nonce: Felt,
    /// Timestamp after which execution is valid.
    pub execute_after: u64,
    /// Timestamp before which execution is valid.
    pub execute_before: u64,
    /// Calls to execute in order.
    pub calls: Vec<Call>,
}

/// Non-standard extension of the [`OutsideExecutionV2`] supported by the Cartridge Controller.
#[derive(Clone, CairoSerde, Serialize, Deserialize, PartialEq, Debug)]
pub struct OutsideExecutionV3 {
    /// Address allowed to initiate execution ('ANY_CALLER' for unrestricted).
    pub caller: ContractAddress,
    /// Nonce with channel (nonce, channel).
    pub nonce: (Felt, u128),
    /// Timestamp after which execution is valid.
    pub execute_after: u64,
    /// Timestamp before which execution is valid.
    pub execute_before: u64,
    /// Calls to execute in order.
    pub calls: Vec<Call>,
}

#[derive(Clone, CairoSerde, Serialize, Deserialize, Debug)]
#[serde(untagged)]
pub enum OutsideExecution {
    /// SNIP-9 standard version.
    V2(OutsideExecutionV2),
    /// Cartridge/Controller extended version.
    V3(OutsideExecutionV3),
}

impl OutsideExecution {
    pub fn caller(&self) -> ContractAddress {
        match self {
            OutsideExecution::V2(v2) => v2.caller,
            OutsideExecution::V3(v3) => v3.caller,
        }
    }
}
