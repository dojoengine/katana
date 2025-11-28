use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use blockifier::execution::contract_class::{CompiledClassV0, CompiledClassV0Inner};
use cairo_vm::serde::deserialize_program::{ApTracking, Member};
use cairo_vm::types::builtin_name::BuiltinName;
use cairo_vm::types::relocatable::MaybeRelocatable;
use cairo_vm::utils::CAIRO_PRIME;
use num_bigint::BigInt;
use num_traits::float::FloatCore;
use num_traits::Num;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Number, Value};
use serde_json_pythonic::to_string_pythonic;
use starknet::core::utils::cairo_short_string_to_felt;
use starknet_api::contract_class::EntryPointType;
use starknet_api::deprecated_contract_class::EntryPointV0;
use starknet_types_core::hash::{Pedersen, StarkHash};

pub use cairo_vm::types::errors::program_errors::ProgramError;
pub use starknet_api::deprecated_contract_class::ContractClassAbiEntry;

use super::ClassHash;
use crate::utils::starknet_keccak;
use crate::Felt;

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize, Default)]
pub struct ContractClass {
    #[serde(default)]
    pub abi: Option<Vec<ContractClassAbiEntry>>,
    pub program: Program,
    pub entry_points_by_type: ContractEntryPoints,
}

impl ContractClass {
    /// Computes the class hash of the legacy (Cairo 0) class.
    pub fn hash(&self) -> ClassHash {
        const API_VERSION: Felt = Felt::ZERO;

        let mut elements = vec![API_VERSION];

        // Hashes external entry points
        elements.push(legacy_entrypoints_hash(&self.entry_points_by_type.external));
        // Hashes l1 handler entry points
        elements.push(legacy_entrypoints_hash(&self.entry_points_by_type.l1_handler));
        // Hashes constructor entry points
        elements.push(legacy_entrypoints_hash(&self.entry_points_by_type.constructor));

        fn builtins_hash(builtins: &[BuiltinName]) -> Felt {
            let mut hasher = starknet_crypto::PedersenHasher::new();
            for builtin in builtins.iter().map(|b| b.to_str()) {
                hasher.update(cairo_short_string_to_felt(builtin).unwrap());
            }
            hasher.finalize()
        }

        // Hashes builtins
        elements.push(builtins_hash(&self.program.builtins));
        // Hashes hinted_class_hash
        elements.push(self.hinted_class_hash());
        // Hashes bytecode
        elements.push(Pedersen::hash_array(&self.program.data));

        Pedersen::hash_array(&elements)
    }

    /// Computes the "hinted" class hash of the legacy (Cairo 0) class.
    ///
    /// This is known as the "hinted" hash as it isn't possible to directly calculate, and thus
    /// prove the correctness of, this hash, since it involves JSON serialization. Instead, this
    /// hash is always calculated outside of the Cairo VM, and then fed to the Cairo program as a
    /// hinted value.
    pub fn hinted_class_hash(&self) -> ClassHash {
        #[derive(serde::Serialize)]
        struct ContractArtifactForHash<'a> {
            #[serde(serialize_with = "serialize_abi_for_hinted_hash")]
            abi: &'a Vec<ContractClassAbiEntry>,
            #[serde(serialize_with = "serialize_program_for_hinted_hash")]
            program: &'a Program,
        }

        static EMPTY_ABI: Vec<ContractClassAbiEntry> = Vec::new();
        let abi = self.abi.as_ref().unwrap_or(&EMPTY_ABI);
        let object = ContractArtifactForHash { abi, program: &self.program };

        let serialized = to_string_pythonic(&object).unwrap();
        std::fs::write("hinted_class_hash_output.json", &serialized).unwrap();
        let serialized_bytes = serialized.as_bytes();

        starknet_keccak(serialized_bytes)
    }
}

impl TryFrom<ContractClass> for CompiledClassV0 {
    type Error = cairo_vm::types::errors::program_errors::ProgramError;

    fn try_from(class: ContractClass) -> Result<Self, Self::Error> {
        let entry_points_by_type = HashMap::from_iter([
            (EntryPointType::External, class.entry_points_by_type.external),
            (EntryPointType::L1Handler, class.entry_points_by_type.l1_handler),
            (EntryPointType::Constructor, class.entry_points_by_type.constructor),
        ]);

        Ok(Self(Arc::new(CompiledClassV0Inner {
            program: class.program.try_into()?,
            entry_points_by_type,
        })))
    }
}

/// Cairo 0 [references](https://docs.cairo-lang.org/how_cairo_works/consts.html#references).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Reference {
    pub ap_tracking_data: ApTracking,
    pub pc: u64,
    pub value: String,
}

impl From<Reference> for cairo_vm::serde::deserialize_program::Reference {
    fn from(value: Reference) -> Self {
        Self { ap_tracking_data: value.ap_tracking_data.into(), pc: value.pc, value: value.value }
    }
}

/// Legacy (Cairo 0) program identifiers.
///
/// These are needed mostly to allow Python hints to work, as hints are allowed to reference Cairo
/// identifiers (e.g. variables) by name, which would otherwise be lost during compilation.
#[derive(Debug, Clone, Eq, PartialEq, ::serde::Serialize, ::serde::Deserialize)]
pub struct Identifier {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decorators: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cairo_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub members: Option<BTreeMap<String, Member>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub references: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pc: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination: Option<String>,
    pub r#type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<RawJsonValue>,
}

impl From<Identifier> for cairo_vm::serde::deserialize_program::Identifier {
    fn from(value: Identifier) -> Self {
        Self {
            pc: value.pc.map(|pc| pc as usize),
            type_: Some(value.r#type),
            value: value.value.map(felt_from_number),
            cairo_type: value.cairo_type,
            full_name: value.full_name,
            members: value.members.map(|m| HashMap::from_iter(m.into_iter())),
        }
    }
}

#[derive(Serialize, Deserialize, Default, Debug, PartialEq, Eq, Clone)]
pub struct ReferenceManager {
    pub references: Vec<Reference>,
}

impl From<ReferenceManager> for cairo_vm::serde::deserialize_program::ReferenceManager {
    fn from(value: ReferenceManager) -> Self {
        Self { references: value.references.into_iter().map(|r| r.into()).collect() }
    }
}

#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq, Eq)]
pub struct FlowTrackingData {
    pub ap_tracking: ApTracking,
    pub reference_ids: BTreeMap<String, u64>,
}

impl From<FlowTrackingData> for cairo_vm::serde::deserialize_program::FlowTrackingData {
    fn from(value: FlowTrackingData) -> Self {
        let reference_ids_iter = value.reference_ids.into_iter().map(|(k, v)| (k, v as usize));
        let reference_ids = HashMap::from_iter(reference_ids_iter);
        Self { ap_tracking: value.ap_tracking, reference_ids }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, derive_more::From, ::serde::Deserialize)]
pub struct HintParams {
    pub code: String,
    pub accessible_scopes: Vec<String>,
    pub flow_tracking_data: FlowTrackingData,
}

impl From<HintParams> for cairo_vm::serde::deserialize_program::HintParams {
    fn from(value: HintParams) -> Self {
        let reference_ids_iter =
            value.flow_tracking_data.reference_ids.into_iter().map(|(k, v)| (k, v as usize));
        let reference_ids = HashMap::from_iter(reference_ids_iter);

        Self {
            code: value.code,
            accessible_scopes: value.accessible_scopes,
            flow_tracking_data: cairo_vm::serde::deserialize_program::FlowTrackingData {
                ap_tracking: value.flow_tracking_data.ap_tracking,
                reference_ids,
            },
        }
    }
}

impl Serialize for HintParams {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("HintParams", 3)?;
        state.serialize_field("accessible_scopes", &self.accessible_scopes)?;
        state.serialize_field("code", &self.code)?;
        state.serialize_field("flow_tracking_data", &self.flow_tracking_data)?;
        state.end()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Attribute {
    pub name: String,
    pub start_pc: u64,
    pub end_pc: u64,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flow_tracking_data: Option<FlowTrackingData>,
    #[serde(default)]
    pub accessible_scopes: Vec<String>,
}

impl From<Attribute> for cairo_vm::serde::deserialize_program::Attribute {
    fn from(value: Attribute) -> Self {
        Self {
            name: value.name,
            value: value.value,
            end_pc: value.end_pc as usize,
            start_pc: value.start_pc as usize,
            flow_tracking_data: value.flow_tracking_data.map(|data| data.into()),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, ::serde::Serialize, ::serde::Deserialize, Default)]
pub struct Program {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attributes: Vec<Attribute>,
    pub builtins: Vec<BuiltinName>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compiler_version: Option<String>,
    pub data: Vec<Felt>,
    #[serde(default)]
    pub debug_info: Option<serde_json::Value>,
    pub hints: BTreeMap<u64, Vec<HintParams>>,
    pub identifiers: BTreeMap<String, Identifier>,
    pub main_scope: String,
    pub prime: String,
    pub reference_manager: ReferenceManager,
}

impl TryFrom<Program> for cairo_vm::types::program::Program {
    type Error = cairo_vm::types::errors::program_errors::ProgramError;

    fn try_from(value: Program) -> Result<Self, Self::Error> {
        let builtins = value.builtins;
        let data = value.data.into_iter().map(MaybeRelocatable::from).collect::<Vec<_>>();
        let main = None;

        let hints =
            HashMap::from_iter(value.hints.into_iter().map(|(k, v)| (k as usize, v.into())));

        let reference_manager = value.reference_manager;
        let instruction_locations = None;

        let identifiers =
            HashMap::from_iter(value.identifiers.into_iter().map(|(k, v)| (k, v.into())));

        let error_message_attributes = value
            .attributes
            .into_iter()
            .filter(|attr| attr.name == "error_message")
            .map(|a| a.into())
            .collect();

        Self::new(
            builtins,
            data,
            main,
            hints,
            reference_manager,
            identifiers,
            error_message_attributes,
            instruction_locations,
        )
    }
}

#[derive(Clone, Default, Debug, PartialEq, Eq, ::serde::Serialize, ::serde::Deserialize)]
pub struct ContractEntryPoints {
    #[serde(rename = "EXTERNAL")]
    pub external: Vec<EntryPointV0>,
    #[serde(rename = "L1_HANDLER")]
    pub l1_handler: Vec<EntryPointV0>,
    #[serde(rename = "CONSTRUCTOR")]
    pub constructor: Vec<EntryPointV0>,
}

/// A wrapper around [`serde_json::value::RawValue`] that preserves the exact JSON representation
/// while implementing `Eq` and `PartialEq` by comparing the raw string content.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RawJsonValue(Box<serde_json::value::RawValue>);

impl PartialEq for RawJsonValue {
    fn eq(&self, other: &Self) -> bool {
        self.0.get() == other.0.get()
    }
}

impl Eq for RawJsonValue {}

/// Serializes legacy ABI entries in a format compatible with starknet-rs hinted class hash.
///
/// This uses the same serialization pattern as starknet-rs: using `#[serde(flatten)]` to put
/// the inner struct fields first, then appending the `type` field at the end.
/// See: https://github.com/xJonathanLEI/starknet-rs/issues/216
fn serialize_abi_for_hinted_hash<S: Serializer>(
    source: &Vec<ContractClassAbiEntry>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    use starknet_api::deprecated_contract_class::{
        ContractClassAbiEntry, EventAbiEntry, StructAbiEntry, TypedParameter,
    };

    let entries = source;

    // Helper struct that puts `type` at the end using flatten, matching starknet-rs behavior.
    #[derive(Serialize)]
    struct TypedValue<'a, T> {
        #[serde(flatten)]
        value: &'a T,
        r#type: &'static str,
    }

    #[derive(Serialize)]
    struct TypedParam<'a> {
        name: &'a String,
        r#type: &'a String,
    }

    #[derive(Serialize)]
    struct StructMember<'a> {
        name: &'a String,
        offset: usize,
        r#type: &'a String,
    }

    // Inner structs without `type` field - will be flattened
    #[derive(Serialize)]
    struct RawConstructor<'a> {
        inputs: Vec<TypedParam<'a>>,
        name: &'a String,
        outputs: Vec<TypedParam<'a>>,
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct RawFunction<'a> {
        inputs: Vec<TypedParam<'a>>,
        name: &'a String,
        outputs: Vec<TypedParam<'a>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        state_mutability:
            Option<&'a starknet_api::deprecated_contract_class::FunctionStateMutability>,
    }

    #[derive(Serialize)]
    struct RawStruct<'a> {
        members: Vec<StructMember<'a>>,
        name: &'a String,
        size: usize,
    }

    #[derive(Serialize)]
    struct RawL1Handler<'a> {
        inputs: Vec<TypedParam<'a>>,
        name: &'a String,
        outputs: Vec<TypedParam<'a>>,
    }

    #[derive(Serialize)]
    struct RawEvent<'a> {
        data: Vec<TypedParam<'a>>,
        keys: Vec<TypedParam<'a>>,
        name: &'a String,
    }

    fn convert_typed_params(params: &[TypedParameter]) -> Vec<TypedParam<'_>> {
        params.iter().map(|p| TypedParam { name: &p.name, r#type: &p.r#type }).collect()
    }

    let mut seq = serializer.serialize_seq(Some(entries.len()))?;

    for entry in entries {
        match entry {
            ContractClassAbiEntry::Constructor(f) => {
                let inputs = convert_typed_params(&f.inputs);
                let outputs = convert_typed_params(&f.outputs);
                let value = RawConstructor { inputs, name: &f.name, outputs };
                seq.serialize_element(&TypedValue { value: &value, r#type: "constructor" })?;
            }
            ContractClassAbiEntry::Function(f) => {
                let inputs = convert_typed_params(&f.inputs);
                let outputs = convert_typed_params(&f.outputs);
                let value = RawFunction {
                    inputs,
                    name: &f.name,
                    outputs,
                    state_mutability: f.state_mutability.as_ref(),
                };
                seq.serialize_element(&TypedValue { value: &value, r#type: "function" })?;
            }
            ContractClassAbiEntry::L1Handler(f) => {
                let inputs = convert_typed_params(&f.inputs);
                let outputs = convert_typed_params(&f.outputs);
                let value = RawL1Handler { inputs, name: &f.name, outputs };
                seq.serialize_element(&TypedValue { value: &value, r#type: "l1_handler" })?;
            }
            ContractClassAbiEntry::Event(EventAbiEntry { data, keys, name, .. }) => {
                let data = convert_typed_params(data);
                let keys = convert_typed_params(keys);
                let value = RawEvent { data, keys, name };
                seq.serialize_element(&TypedValue { value: &value, r#type: "event" })?;
            }
            ContractClassAbiEntry::Struct(StructAbiEntry { members, name, size, .. }) => {
                let members: Vec<_> = members
                    .iter()
                    .map(|m| StructMember { name: &m.name, offset: m.offset, r#type: &m.r#type })
                    .collect();
                let value = RawStruct { members, name, size: *size };
                seq.serialize_element(&TypedValue { value: &value, r#type: "struct" })?;
            }
        }
    }

    seq.end()
}

fn serialize_attribute_for_hinted_hash<S: Serializer>(
    source: &Vec<Attribute>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;

    #[derive(Serialize)]
    struct Helper<'a> {
        #[serde(skip_serializing_if = "Vec::is_empty")]
        accessible_scopes: &'a Vec<String>,
        end_pc: &'a u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        flow_tracking_data: &'a Option<FlowTrackingData>,
        name: &'a String,
        start_pc: &'a u64,
        value: &'a String,
    }

    let mut seq = serializer.serialize_seq(Some(source.len()))?;

    for attribute in source {
        seq.serialize_element(&Helper {
            accessible_scopes: &attribute.accessible_scopes,
            end_pc: &attribute.end_pc,
            flow_tracking_data: &attribute.flow_tracking_data,
            name: &attribute.name,
            start_pc: &attribute.start_pc,
            value: &attribute.value,
        })?;
    }

    seq.end()
}

fn serialize_program_for_hinted_hash<S: Serializer>(
    source: &Program,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    #[derive(::serde::Serialize)]
    struct Helper<'a> {
        #[serde(skip_serializing_if = "Vec::is_empty")]
        #[serde(serialize_with = "serialize_attribute_for_hinted_hash")]
        attributes: &'a Vec<Attribute>,
        builtins: &'a Vec<BuiltinName>,
        #[serde(skip_serializing_if = "Option::is_none")]
        compiler_version: &'a Option<String>,
        data: &'a Vec<Felt>,
        // #[serde(skip_serializing_if = "Option::is_none")]
        debug_info: &'a Option<serde_json::Value>,
        hints: &'a BTreeMap<u64, Vec<HintParams>>,
        identifiers: &'a BTreeMap<String, Identifier>,
        main_scope: &'a String,
        prime: &'a String,
        reference_manager: &'a ReferenceManager,
    }

    if source.compiler_version.is_some() {
        // Anything since 0.10.0 can be hashed directly. No extra overhead incurred.

        Helper::serialize(
            &Helper {
                attributes: &source.attributes,
                builtins: &source.builtins,
                compiler_version: &source.compiler_version,
                data: &source.data,
                debug_info: &None,
                hints: &source.hints,
                identifiers: &source.identifiers,
                main_scope: &source.main_scope,
                prime: &source.prime,
                reference_manager: &source.reference_manager,
            },
            serializer,
        )
    } else {
        // This is needed for backward compatibility with pre-0.10.0 contract artifacts.

        // We're cloning the entire `identifiers` here as a temporary patch. This is not
        // optimal, as it should technically be possible to avoid the cloning. This only
        // affects very old contract artifacts though.
        // TODO: optimize this to remove cloning.

        let patched_identifiers = source
            .identifiers
            .iter()
            .map(|(key, value)| {
                (
                    key.to_owned(),
                    Identifier {
                        decorators: value.decorators.to_owned(),
                        cairo_type: value
                            .cairo_type
                            .to_owned()
                            .map(|content| content.replace(": ", " : ")),
                        full_name: value.full_name.to_owned(),
                        members: value.members.to_owned().map(|map| {
                            map.iter()
                                .map(|(key, value)| {
                                    (
                                        key.to_owned(),
                                        Member {
                                            cairo_type: value.cairo_type.replace(": ", " : "),
                                            offset: value.offset,
                                        },
                                    )
                                })
                                .collect()
                        }),
                        references: value.references.to_owned(),
                        size: value.size,
                        pc: value.pc,
                        destination: value.destination.to_owned(),
                        r#type: value.r#type.to_owned(),
                        value: value.value.to_owned(),
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();

        Helper::serialize(
            &Helper {
                attributes: &source.attributes,
                builtins: &source.builtins,
                compiler_version: &source.compiler_version,
                data: &source.data,
                debug_info: &None,
                hints: &source.hints,
                identifiers: &patched_identifiers,
                main_scope: &source.main_scope,
                prime: &source.prime,
                reference_manager: &source.reference_manager,
            },
            serializer,
        )
    }
}

fn legacy_entrypoints_hash(entrypoints: &[EntryPointV0]) -> Felt {
    let mut hasher = starknet_crypto::PedersenHasher::new();
    for entry in entrypoints {
        hasher.update(entry.selector.0);
        hasher.update(entry.offset.0.into());
    }
    hasher.finalize()
}

fn felt_from_number(value: RawJsonValue) -> Felt {
    let n = serde_json::Number::deserialize(&*value.0).unwrap();
    match Felt::from_dec_str(&n.to_string()).ok() {
        Some(x) => x,
        // Handle de Number with scientific notation cases
        // e.g.: n = Number(1e27)
        None => deserialize_scientific_notation(n).unwrap(),
    }
}

fn deserialize_scientific_notation(n: serde_json::Number) -> Option<Felt> {
    match n.as_f64() {
        None => {
            let str = n.to_string();
            let list: [&str; 2] = str.split('e').collect::<Vec<&str>>().try_into().ok()?;
            let exponent = list[1].parse::<u128>().ok()?;

            // Apply % CAIRO_PRIME, BECAUSE Felt::from_dec_str fails with big numbers
            let prime_bigint = BigInt::from_biguint(num_bigint::Sign::Plus, CAIRO_PRIME.clone());
            let base_bigint = BigInt::from_str_radix(list[0], 10).ok()? % prime_bigint;
            let base = Felt::from_dec_str(&base_bigint.to_string()).ok()?;

            Some(base * Felt::from(10).pow(exponent))
        }
        Some(float) => {
            // Apply % CAIRO_PRIME, BECAUSE Felt::from_dec_str fails with big numbers
            let prime_bigint = BigInt::from_biguint(num_bigint::Sign::Plus, CAIRO_PRIME.clone());
            let number = BigInt::from_str_radix(&FloatCore::round(float).to_string(), 10).ok()?
                % prime_bigint;
            Felt::from_dec_str(&number.to_string()).ok()
        }
    }
}

#[cfg(test)]
mod tests {
    use starknet::core::types::contract::legacy::LegacyContractClass as StarknetRsLegacyContractClass;

    // Compare it against the hash computed using `starknet-rs` types
    #[test]
    fn compute_legacy_class_hash() {
        let artifact = include_str!("../../../contracts/build/legacy/erc20.json");

        let class = serde_json::from_str::<super::ContractClass>(artifact).unwrap();
        let actual_hash = class.hash();
        let actual_hinted_hash = class.hinted_class_hash();

        let class = serde_json::from_str::<StarknetRsLegacyContractClass>(artifact).unwrap();
        let expected_hash = class.class_hash().unwrap();
        let expected_hinted_hash = class.hinted_class_hash().unwrap();

        assert_eq!(actual_hinted_hash, expected_hinted_hash);
        assert_eq!(actual_hash, expected_hash);
    }
}
