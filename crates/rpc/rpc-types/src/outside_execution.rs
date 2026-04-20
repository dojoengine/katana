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
//! matching version. This is why [`OutsideExecution`] is not deriving `CairoSerde` directly.
//!
//! <https://github.com/cartridge-gg/argent-contracts-starknet/blob/35f21a533e7636f926484546652fb3470d2d478d/src/outside_execution/interface.cairo#L38>

use cainome::cairo_serde::{deserialize_from_hex, serialize_as_hex};
use cainome::cairo_serde_derive::CairoSerde;
use cainome_cairo_serde::CairoSerde;
use katana_primitives::execution::Call;
use katana_primitives::{ContractAddress, Felt};
use serde::{Deserialize, Serialize};
use starknet::macros::{selector, short_string};
use starknet_crypto::poseidon_hash_many;

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

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq, derive_more::From)]
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

trait StructHashRev1 {
    const TYPE_HASH_REV_1: Felt;
    fn get_struct_hash_rev_1(&self) -> Felt;
}

pub trait MessageHashRev1 {
    fn get_message_hash_rev_1(&self, chain_id: Felt, contract_address: ContractAddress) -> Felt;
}

struct StarknetDomain {
    name: Felt,
    version: Felt,
    chain_id: Felt,
    revision: Felt,
}

impl StructHashRev1 for StarknetDomain {
    const TYPE_HASH_REV_1: Felt = selector!(
        "\"StarknetDomain\"(\"name\":\"shortstring\",\"version\":\"shortstring\",\"chainId\":\"\
         shortstring\",\"revision\":\"shortstring\")"
    );

    fn get_struct_hash_rev_1(&self) -> Felt {
        poseidon_hash_many(&[
            Self::TYPE_HASH_REV_1,
            self.name,
            self.version,
            self.chain_id,
            self.revision,
        ])
    }
}

impl StructHashRev1 for Call {
    const TYPE_HASH_REV_1: Felt = selector!(
        "\"Call\"(\"To\":\"ContractAddress\",\"Selector\":\"selector\",\"Calldata\":\"felt*\")"
    );

    fn get_struct_hash_rev_1(&self) -> Felt {
        poseidon_hash_many(&[
            Self::TYPE_HASH_REV_1,
            self.contract_address.into(),
            self.entry_point_selector,
            poseidon_hash_many(&self.calldata),
        ])
    }
}

impl StructHashRev1 for OutsideExecutionV2 {
    const TYPE_HASH_REV_1: Felt = selector!(
        "\"OutsideExecution\"(\"Caller\":\"ContractAddress\",\"Nonce\":\"felt\",\"Execute \
         After\":\"u128\",\"Execute \
         Before\":\"u128\",\"Calls\":\"Call*\")\"Call\"(\"To\":\"ContractAddress\",\"Selector\":\"\
         selector\",\"Calldata\":\"felt*\")"
    );

    fn get_struct_hash_rev_1(&self) -> Felt {
        let hashed_calls =
            self.calls.iter().map(StructHashRev1::get_struct_hash_rev_1).collect::<Vec<_>>();
        poseidon_hash_many(&[
            Self::TYPE_HASH_REV_1,
            self.caller.into(),
            self.nonce,
            self.execute_after.into(),
            self.execute_before.into(),
            poseidon_hash_many(&hashed_calls),
        ])
    }
}

impl MessageHashRev1 for OutsideExecutionV2 {
    fn get_message_hash_rev_1(&self, chain_id: Felt, contract_address: ContractAddress) -> Felt {
        // Version and Revision should be shortstring '1' and not felt 1 for SNIP-9 due to a
        // mistake in the Braavos contracts and has been copied for compatibility.
        // Revision will also be a number for all SNIP12-rev1 signatures because of the same issue.
        let domain = StarknetDomain {
            name: short_string!("Account.execute_from_outside"),
            version: Felt::TWO,
            chain_id,
            revision: Felt::ONE,
        };
        poseidon_hash_many(&[
            short_string!("StarkNet Message"),
            domain.get_struct_hash_rev_1(),
            contract_address.into(),
            self.get_struct_hash_rev_1(),
        ])
    }
}

impl StructHashRev1 for OutsideExecutionV3 {
    const TYPE_HASH_REV_1: Felt = selector!(
        "\"OutsideExecution\"(\"Caller\":\"ContractAddress\",\"Nonce\":\"(felt,u128)\",\"Execute \
         After\":\"u128\",\"Execute \
         Before\":\"u128\",\"Calls\":\"Call*\")\"Call\"(\"To\":\"ContractAddress\",\"Selector\":\"\
         selector\",\"Calldata\":\"felt*\")"
    );

    fn get_struct_hash_rev_1(&self) -> Felt {
        let hashed_calls =
            self.calls.iter().map(StructHashRev1::get_struct_hash_rev_1).collect::<Vec<_>>();
        poseidon_hash_many(&[
            Self::TYPE_HASH_REV_1,
            self.caller.into(),
            self.nonce.0,
            self.nonce.1.into(),
            self.execute_after.into(),
            self.execute_before.into(),
            poseidon_hash_many(&hashed_calls),
        ])
    }
}

impl MessageHashRev1 for OutsideExecutionV3 {
    fn get_message_hash_rev_1(&self, chain_id: Felt, contract_address: ContractAddress) -> Felt {
        let domain = StarknetDomain {
            name: short_string!("Account.execute_from_outside"),
            version: Felt::TWO,
            chain_id,
            revision: Felt::TWO,
        };
        poseidon_hash_many(&[
            short_string!("StarkNet Message"),
            domain.get_struct_hash_rev_1(),
            contract_address.into(),
            self.get_struct_hash_rev_1(),
        ])
    }
}

impl MessageHashRev1 for OutsideExecution {
    fn get_message_hash_rev_1(&self, chain_id: Felt, contract_address: ContractAddress) -> Felt {
        match self {
            OutsideExecution::V2(v2) => v2.get_message_hash_rev_1(chain_id, contract_address),
            OutsideExecution::V3(v3) => v3.get_message_hash_rev_1(chain_id, contract_address),
        }
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

    use cainome_cairo_serde::CairoSerde;
    use katana_primitives::{address, felt, ContractAddress, Felt};
    use serde_json::json;
    use starknet::macros::selector;

    use crate::outside_execution::{
        Call, MessageHashRev1, NonceChannel, OutsideExecutionV2, OutsideExecutionV3,
    };
    use crate::OutsideExecution;

    #[test]
    fn outside_execution_v2_cairo_serialization() {
        let expected_deserialized = OutsideExecutionV2 {
            caller: address!("0x414e595f43414c4c4552"),
            nonce: felt!("0x6716dcf8796086bd5a2db25d87e99b1f14e96caa105d54823bcc6c3fe01561"),
            execute_after: 0x0,
            execute_before: 0xb2d05e00,
            calls: vec![Call {
                contract_address: address!(
                    "0x1153499afc678b92c825c86219d742f86c9385465c64aeb41a950e2ee34b1fd"
                ),
                entry_point_selector: felt!(
                    "0x2d1af4265f4530c75b41282ed3b71617d3d435e96fe13b08848482173692f4f"
                ),
                calldata: vec![felt!("0x14d"), felt!("0x1")],
            }],
        };

        let expected_serialized = vec![
            felt!("0x414e595f43414c4c4552"),
            felt!("0x6716dcf8796086bd5a2db25d87e99b1f14e96caa105d54823bcc6c3fe01561"),
            felt!("0x0"),
            felt!("0xb2d05e00"),
            felt!("0x1"),
            felt!("0x1153499afc678b92c825c86219d742f86c9385465c64aeb41a950e2ee34b1fd"),
            felt!("0x2d1af4265f4530c75b41282ed3b71617d3d435e96fe13b08848482173692f4f"),
            felt!("0x2"),
            felt!("0x14d"),
            felt!("0x1"),
        ];

        // serialize directly from OutsideExecutionV2
        let serialized = OutsideExecutionV2::cairo_serialize(&expected_deserialized);
        assert_eq!(serialized, expected_serialized);

        // serialize from OutsideExecutionV3
        let serialized = OutsideExecution::cairo_serialize(&expected_deserialized.clone().into());
        assert_eq!(serialized, expected_serialized);

        // deserialize directly from OutsideExecutionV2
        let deserialized = OutsideExecutionV2::cairo_deserialize(&expected_serialized, 0).unwrap();
        assert_eq!(deserialized, expected_deserialized);

        // deserialize from OutsideExecution enum
        let deserialized = OutsideExecution::cairo_deserialize(&expected_serialized, 0).unwrap();
        assert_eq!(deserialized, expected_deserialized.into());
    }

    #[test]
    fn outside_execution_v3_cairo_serialization() {
        let expected_deserialized = OutsideExecutionV3 {
            caller: address!("0x414e595f43414c4c4552"),
            nonce: NonceChannel(
                felt!("0x4e0120d114cb35ab264eb450058eebfef6393337f23291b99979c2a65df02b2"),
                0x1,
            ),
            execute_after: 0x0,
            execute_before: 0x69d6d324,
            calls: vec![Call {
                contract_address: address!(
                    "0xcce923b653e892b5e4cce256770df005a5dcf9812ff825b66cf5bc41136f15"
                ),
                entry_point_selector: felt!(
                    "0xf2f7c15cbe06c8d94597cd91fd7f3369eae842359235712def5584f8d270cd"
                ),
                calldata: vec![felt!(
                    "0x743c83c41ce99ad470aa308823f417b2141e02e04571f5c0004e743556e7faf"
                )],
            }],
        };

        let expected_serialized = vec![
            felt!("0x414e595f43414c4c4552"),
            felt!("0x4e0120d114cb35ab264eb450058eebfef6393337f23291b99979c2a65df02b2"),
            felt!("0x1"),
            felt!("0x0"),
            felt!("0x69d6d324"),
            felt!("0x1"),
            felt!("0xcce923b653e892b5e4cce256770df005a5dcf9812ff825b66cf5bc41136f15"),
            felt!("0xf2f7c15cbe06c8d94597cd91fd7f3369eae842359235712def5584f8d270cd"),
            felt!("0x1"),
            felt!("0x743c83c41ce99ad470aa308823f417b2141e02e04571f5c0004e743556e7faf"),
        ];

        // serialize directly from OutsideExecutionV3
        let serialized = OutsideExecutionV3::cairo_serialize(&expected_deserialized);
        assert_eq!(serialized, expected_serialized);

        // serialize from OutsideExecutionV3
        let serialized = OutsideExecution::cairo_serialize(&expected_deserialized.clone().into());
        assert_eq!(serialized, expected_serialized);

        // deserialize directly from OutsideExecutionV3
        let deserialized = OutsideExecutionV3::cairo_deserialize(&expected_serialized, 0).unwrap();
        assert_eq!(deserialized, expected_deserialized);

        // deserialize from OutsideExecution enum
        let deserialized = OutsideExecution::cairo_deserialize(&expected_serialized, 0).unwrap();
        assert_eq!(deserialized, expected_deserialized.into());
    }

    #[test]
    fn outside_execution_v2_json_serialization() {
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
    fn outside_execution_v3_json_serialization() {
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

    #[test]
    fn outside_execution_v2_message_hash_rev_1() {
        let outside_execution = OutsideExecutionV2 {
            caller: address!("0x414e595f43414c4c4552"),
            nonce: felt!("0x6716dcf8796086bd5a2db25d87e99b1f14e96caa105d54823bcc6c3fe01561"),
            execute_after: 0,
            execute_before: 0xb2d05e00,
            calls: vec![Call {
                contract_address: address!(
                    "0x1153499afc678b92c825c86219d742f86c9385465c64aeb41a950e2ee34b1fd"
                ),
                entry_point_selector: felt!(
                    "0x2d1af4265f4530c75b41282ed3b71617d3d435e96fe13b08848482173692f4f"
                ),
                calldata: vec![felt!("0x14d"), felt!("0x1")],
            }],
        };

        let chain_id = felt!("0x534e5f5345504f4c4941"); // SN_SEPOLIA
        let contract_address = address!("0xdeadbeef");

        let hash = outside_execution.get_message_hash_rev_1(chain_id, contract_address);

        // Dispatching through the enum must produce the same hash.
        let wrapped: OutsideExecution = outside_execution.clone().into();
        assert_eq!(hash, wrapped.get_message_hash_rev_1(chain_id, contract_address));

        // Distinct inputs produce distinct hashes.
        assert_ne!(hash, outside_execution.get_message_hash_rev_1(chain_id, ContractAddress::ZERO));
        assert_ne!(hash, outside_execution.get_message_hash_rev_1(Felt::ZERO, contract_address));
    }

    #[test]
    fn outside_execution_v3_message_hash_rev_1() {
        let outside_execution = OutsideExecutionV3 {
            caller: address!("0x414e595f43414c4c4552"),
            nonce: NonceChannel(
                felt!("0x4e0120d114cb35ab264eb450058eebfef6393337f23291b99979c2a65df02b2"),
                1,
            ),
            execute_after: 0,
            execute_before: 0x69d6d324,
            calls: vec![Call {
                contract_address: address!(
                    "0xcce923b653e892b5e4cce256770df005a5dcf9812ff825b66cf5bc41136f15"
                ),
                entry_point_selector: felt!(
                    "0xf2f7c15cbe06c8d94597cd91fd7f3369eae842359235712def5584f8d270cd"
                ),
                calldata: vec![felt!(
                    "0x743c83c41ce99ad470aa308823f417b2141e02e04571f5c0004e743556e7faf"
                )],
            }],
        };

        let chain_id = felt!("0x534e5f5345504f4c4941");
        let contract_address = address!("0xdeadbeef");

        let hash = outside_execution.get_message_hash_rev_1(chain_id, contract_address);

        let wrapped: OutsideExecution = outside_execution.clone().into();
        assert_eq!(hash, wrapped.get_message_hash_rev_1(chain_id, contract_address));

        // V3 uses a different domain revision than V2, so the hashes must differ even
        // when the shared fields are equal.
        let v2 = OutsideExecutionV2 {
            caller: outside_execution.caller,
            nonce: outside_execution.nonce.0,
            execute_after: outside_execution.execute_after,
            execute_before: outside_execution.execute_before,
            calls: outside_execution.calls.clone(),
        };
        assert_ne!(hash, v2.get_message_hash_rev_1(chain_id, contract_address));
    }
}
