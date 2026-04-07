//! Outside execution types for Starknet accounts.
//!
//! Outside execution (meta-transactions) allows protocols to submit transactions
//! on behalf of user accounts with their signatures. This enables delayed orders,
//! fee subsidy, and other advanced transaction patterns.
//!
//! Based on [SNIP-9](https://github.com/starknet-io/SNIPs/blob/main/SNIPS/snip-9.md).
//!
//! An important note is that the `execute_from_outside_[v2/v3]` functions are not expecting
//! the serialized enum [`OutsideExecution`] but instead the variant already serialized for the
//! matching version.
//! This is why [`OutsideExecution`] is not deriving `CairoSerde` directly.
//! <https://github.com/cartridge-gg/argent-contracts-starknet/blob/35f21a533e7636f926484546652fb3470d2d478d/src/outside_execution/interface.cairo#L38>

use cainome::cairo_serde::{deserialize_from_hex, serialize_as_hex};
use cainome::cairo_serde_derive::CairoSerde;
use cainome_cairo_serde::CairoSerde;
use katana_primitives::execution::Call;
use katana_primitives::{ContractAddress, Felt};
use serde::{Deserialize, Serialize};
use starknet::macros::selector;

/// Nonce channel
#[derive(Clone, CairoSerde, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct NonceChannel(
    Felt,
    #[serde(serialize_with = "serialize_as_hex", deserialize_with = "deserialize_from_hex")] u128,
);

/// Outside execution version 2 (SNIP-9 standard).
#[derive(Clone, CairoSerde, Serialize, Deserialize, PartialEq, Eq, Debug)]
pub struct OutsideExecutionV2 {
    /// Address allowed to initiate execution ('ANY_CALLER' for unrestricted).
    pub caller: ContractAddress,
    /// Unique nonce to prevent signature reuse.
    pub nonce: Felt,
    /// Timestamp after which execution is valid.
    #[serde(serialize_with = "serialize_as_hex", deserialize_with = "deserialize_from_hex")]
    pub execute_after: u64,
    /// Timestamp before which execution is valid.
    #[serde(serialize_with = "serialize_as_hex", deserialize_with = "deserialize_from_hex")]
    pub execute_before: u64,
    /// Calls to execute in order.
    #[serde(with = "calls_serde")]
    pub calls: Vec<Call>,
}

/// Non-standard extension of the [`OutsideExecutionV2`] supported by the Cartridge Controller.
#[derive(Clone, CairoSerde, Serialize, Deserialize, PartialEq, Eq, Debug)]
pub struct OutsideExecutionV3 {
    /// Address allowed to initiate execution ('ANY_CALLER' for unrestricted).
    pub caller: ContractAddress,
    /// Nonce.
    pub nonce: NonceChannel,
    /// Timestamp after which execution is valid.
    #[serde(serialize_with = "serialize_as_hex", deserialize_with = "deserialize_from_hex")]
    pub execute_after: u64,
    /// Timestamp before which execution is valid.
    #[serde(serialize_with = "serialize_as_hex", deserialize_with = "deserialize_from_hex")]
    pub execute_before: u64,
    /// Calls to execute in order.
    #[serde(with = "calls_serde")]
    pub calls: Vec<Call>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
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

    pub fn calls(&self) -> &[Call] {
        match self {
            Self::V2(v) => &v.calls,
            Self::V3(v) => &v.calls,
        }
    }

    pub fn selector(&self) -> Felt {
        match self {
            Self::V2(_) => selector!("execute_from_outside_v2"),
            Self::V3(_) => selector!("execute_from_outside_v3"),
        }
    }
}

impl CairoSerde for OutsideExecution {
    const SERIALIZED_SIZE: Option<usize> = None;

    type RustType = Self;

    fn cairo_serialized_size(rust: &Self::RustType) -> usize {
        match rust {
            OutsideExecution::V2(v2) => OutsideExecutionV2::cairo_serialized_size(v2),
            OutsideExecution::V3(v3) => OutsideExecutionV3::cairo_serialized_size(v3),
        }
    }

    fn cairo_serialize(rust: &Self::RustType) -> Vec<Felt> {
        match rust {
            OutsideExecution::V2(v2) => OutsideExecutionV2::cairo_serialize(v2),
            OutsideExecution::V3(v3) => OutsideExecutionV3::cairo_serialize(v3),
        }
    }

    fn cairo_deserialize(
        felts: &[Felt],
        offset: usize,
    ) -> Result<Self::RustType, ::cainome_cairo_serde::Error> {
        if let Ok(value) = OutsideExecutionV2::cairo_deserialize(felts, offset) {
            return Ok(OutsideExecution::V2(value));
        }

        if let Ok(value) = OutsideExecutionV3::cairo_deserialize(felts, offset) {
            return Ok(OutsideExecution::V3(value));
        }

        Err(::cainome_cairo_serde::Error::Deserialize(
            "unknown outside execution variant".to_string(),
        ))
    }
}

/// An outside execution request that has been signed by the caller.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignedOutsideExecution {
    /// The contract address of the caller.
    pub address: ContractAddress,
    /// The outside execution request to be executed.
    pub outside_execution: OutsideExecution,
    /// The signature of the caller.
    pub signature: Vec<Felt>,
}

impl From<SignedOutsideExecution> for Call {
    fn from(signed: SignedOutsideExecution) -> Self {
        let SignedOutsideExecution { address: contract_address, outside_execution, signature } =
            signed;

        let entry_point_selector = outside_execution.selector();

        let outside_execution = OutsideExecution::cairo_serialize(&outside_execution);
        let signature = Vec::<Felt>::cairo_serialize(&signature);
        let calldata = [outside_execution, signature].concat();

        Self { contract_address, calldata, entry_point_selector }
    }
}

mod calls_serde {
    use katana_primitives::execution::Call;
    use katana_primitives::{ContractAddress, Felt};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize)]
    struct CallRef<'a> {
        #[serde(rename = "to")]
        contract_address: &'a ContractAddress,
        #[serde(rename = "selector")]
        entry_point_selector: &'a Felt,
        calldata: &'a Vec<Felt>,
    }

    #[derive(Deserialize)]
    struct CallDe {
        #[serde(rename = "to")]
        contract_address: ContractAddress,
        #[serde(rename = "selector")]
        entry_point_selector: Felt,
        calldata: Vec<Felt>,
    }

    pub fn serialize<S: Serializer>(calls: &[Call], serializer: S) -> Result<S::Ok, S::Error> {
        let refs: Vec<CallRef<'_>> = calls
            .iter()
            .map(|c| CallRef {
                contract_address: &c.contract_address,
                entry_point_selector: &c.entry_point_selector,
                calldata: &c.calldata,
            })
            .collect();
        refs.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Call>, D::Error> {
        let items = Vec::<CallDe>::deserialize(deserializer)?;
        Ok(items
            .into_iter()
            .map(|c| Call {
                contract_address: c.contract_address,
                entry_point_selector: c.entry_point_selector,
                calldata: c.calldata,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {

    use katana_primitives::{address, felt, Felt};
    use serde_json::json;
    use starknet::macros::selector;

    use crate::outside_execution::{Call, NonceChannel, OutsideExecutionV2, OutsideExecutionV3};

    #[test]
    fn outside_execution_v2_serialization() {
        let outside_execution = OutsideExecutionV2 {
            caller: address!("0x414e595f43414c4c4552"),
            execute_after: 0,
            execute_before: 3000000000,
            calls: vec![
                Call {
                    contract_address: address!(
                        "0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7"
                    ),
                    entry_point_selector: selector!("approve"),
                    calldata: vec![
                        felt!("0x50302d9f4df7a96567423f64f1271ef07537469d8e8c4dd2409cf3cc4274de4"),
                        felt!("0x11c37937e08000"),
                        Felt::ZERO,
                    ],
                },
                Call {
                    contract_address: address!(
                        "0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7"
                    ),
                    entry_point_selector: selector!("transfer"),
                    calldata: vec![
                        felt!("0x50302d9f4df7a96567423f64f1271ef07537469d8e8c4dd2409cf3cc4274de4"),
                        felt!("0x11c37937e08000"),
                        Felt::ZERO,
                    ],
                },
            ],
            nonce: felt!("0x564b73282b2fb5f201cf2070bf0ca2526871cb7daa06e0e805521ef5d907b33"),
        };

        let serialized = serde_json::to_value(outside_execution).unwrap();

        let expected = json!({
            "caller": "0x414e595f43414c4c4552",
            "execute_after": "0x0",
            "execute_before": "0xb2d05e00",
            "calls": [
                {
                    "to": "0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7",
                    "selector": "0x219209e083275171774dab1df80982e9df2096516f06319c5c6d71ae0a8480c",
                    "calldata": [
                        "0x50302d9f4df7a96567423f64f1271ef07537469d8e8c4dd2409cf3cc4274de4",
                        "0x11c37937e08000",
                        "0x0"
                    ]
                },
                {
                    "to": "0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7",
                    "selector": "0x83afd3f4caedc6eebf44246fe54e38c95e3179a5ec9ea81740eca5b482d12e",
                    "calldata": [
                        "0x50302d9f4df7a96567423f64f1271ef07537469d8e8c4dd2409cf3cc4274de4",
                        "0x11c37937e08000",
                        "0x0"
                    ]
                }
            ],
            "nonce": "0x564b73282b2fb5f201cf2070bf0ca2526871cb7daa06e0e805521ef5d907b33",
        });

        similar_asserts::assert_eq!(serialized, expected);
    }

    #[test]
    fn outside_execution_v3_serialization() {
        let outside_execution = OutsideExecutionV3 {
            caller: address!("0x414e595f43414c4c4552"),
            execute_after: 0,
            execute_before: 3000000000,
            calls: vec![
                Call {
                    contract_address: address!(
                        "0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7"
                    ),
                    entry_point_selector: selector!("approve"),
                    calldata: vec![
                        felt!("0x50302d9f4df7a96567423f64f1271ef07537469d8e8c4dd2409cf3cc4274de4"),
                        felt!("0x11c37937e08000"),
                        Felt::ZERO,
                    ],
                },
                Call {
                    contract_address: address!(
                        "0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7"
                    ),
                    entry_point_selector: selector!("transfer"),
                    calldata: vec![
                        felt!("0x50302d9f4df7a96567423f64f1271ef07537469d8e8c4dd2409cf3cc4274de4"),
                        felt!("0x11c37937e08000"),
                        Felt::ZERO,
                    ],
                },
            ],
            nonce: NonceChannel(
                felt!("0x564b73282b2fb5f201cf2070bf0ca2526871cb7daa06e0e805521ef5d907b33"),
                10,
            ),
        };

        let serialized = serde_json::to_value(outside_execution).unwrap();

        let expected = json!({
            "caller": "0x414e595f43414c4c4552",
            "execute_after": "0x0",
            "execute_before": "0xb2d05e00",
            "calls": [
                {
                    "to": "0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7",
                    "selector": "0x219209e083275171774dab1df80982e9df2096516f06319c5c6d71ae0a8480c",
                    "calldata": [
                        "0x50302d9f4df7a96567423f64f1271ef07537469d8e8c4dd2409cf3cc4274de4",
                        "0x11c37937e08000",
                        "0x0"
                    ]
                },
                {
                    "to": "0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7",
                    "selector": "0x83afd3f4caedc6eebf44246fe54e38c95e3179a5ec9ea81740eca5b482d12e",
                    "calldata": [
                        "0x50302d9f4df7a96567423f64f1271ef07537469d8e8c4dd2409cf3cc4274de4",
                        "0x11c37937e08000",
                        "0x0"
                    ]
                }
            ],
            "nonce": ["0x564b73282b2fb5f201cf2070bf0ca2526871cb7daa06e0e805521ef5d907b33", "0xa"],
        });

        similar_asserts::assert_eq!(serialized, expected);
    }
}
