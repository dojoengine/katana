use std::collections::BTreeMap;
use std::str::FromStr;

use cairo_lang_starknet_classes::casm_contract_class::StarknetSierraCompilationError;
use cairo_lang_starknet_classes::contract_class::{
    version_id_from_serialized_sierra_program, ContractEntryPoint, ContractEntryPoints,
};
use cairo_lang_utils::bigint::BigUintAsHex;
use cairo_vm::serde::deserialize_program::{
    ApTracking,
    FlowTrackingData,
    HintParams,
    Member,
    ReferenceManager,
    // Identifier,
};
use cairo_vm::types::builtin_name::BuiltinName;
use serde::{Deserialize, Serialize, Serializer};
use serde_json_pythonic::to_string_pythonic;
use starknet::macros::short_string;
use starknet_api::contract_class::SierraVersion;
use starknet_api::deprecated_contract_class::EntryPointV0;
use starknet_types_core::hash::{Pedersen, Poseidon, StarkHash};

use crate::utils::{normalize_address, starknet_keccak};
use crate::Felt;

pub type LegacyContractEntryPoint = EntryPointV0;

/// The canonical hash of a contract class. This is the identifier of a class.
pub type ClassHash = Felt;
/// The hash of a compiled contract class.
pub type CompiledClassHash = Felt;

/// The canonical legacy class (Cairo 0) type.
pub type LegacyContractClass = starknet_api::deprecated_contract_class::ContractClass;

pub type LegacyContractClassAbiEntry =
    starknet_api::deprecated_contract_class::ContractClassAbiEntry;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
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

/// Cairo 0 [references](https://docs.cairo-lang.org/how_cairo_works/consts.html#references).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reference {
    pub ap_tracking_data: ApTracking,
    pub pc: u64,
    pub value: String,
}

/// Legacy (Cairo 0) program identifiers.
///
/// These are needed mostly to allow Python hints to work, as hints are allowed to reference Cairo
/// identifiers (e.g. variables) by name, which would otherwise be lost during compilation.
#[derive(Debug, Clone, ::serde::Serialize, ::serde::Deserialize)]
pub struct Identifier {
    pub r#type: String,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Box<serde_json::value::RawValue>>,
}

#[derive(Debug, Clone, ::serde::Serialize, ::serde::Deserialize)]
pub struct LegacyProgram {
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

#[derive(Clone, Default, Debug, PartialEq, Eq, ::serde::Serialize, ::serde::Deserialize)]
pub struct LegacyContractEntryPoints {
    #[serde(rename = "EXTERNAL")]
    pub external: Vec<LegacyContractEntryPoint>,
    #[serde(rename = "L1_HANDLER")]
    pub l1_handler: Vec<LegacyContractEntryPoint>,
    #[serde(rename = "CONSTRUCTOR")]
    pub constructor: Vec<LegacyContractEntryPoint>,
}

pub struct LegacyContractClass2 {
    pub abi: Option<Vec<LegacyContractClassAbiEntry>>,
    pub program: LegacyProgram,
    pub entry_points_by_type: LegacyContractEntryPoints,
}

impl LegacyContractClass2 {
    /// Computes the class hash of the legacy (Cairo 0) class.
    pub fn hash(&self) -> ClassHash {
        const API_VERSION: Felt = Felt::ZERO;

        let mut elements = Vec::new();
        elements.push(API_VERSION);

        // Hashes external entry points
        elements.push(legacy_entrypoints_hash(&self.entry_points_by_type.external));
        // Hashes l1 handler entry points
        elements.push(legacy_entrypoints_hash(&self.entry_points_by_type.l1_handler));
        // Hashes constructor entry points
        elements.push(legacy_entrypoints_hash(&self.entry_points_by_type.constructor));

        fn builtins_hash(builtins: &[BuiltinName]) -> Felt {
            let mut hasher = starknet_crypto::PedersenHasher::new();
            for builtin in builtins.iter().map(|b| b.to_str()) {
                hasher.update(Felt::from_str(builtin).unwrap());
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
            abi: &'a Vec<LegacyContractClassAbiEntry>,
            #[serde(serialize_with = "serialize_program_for_hinted_hash")]
            program: &'a LegacyProgram,
        }

        static EMPTY_ABI: Vec<LegacyContractClassAbiEntry> = Vec::new();
        let abi = self.abi.as_ref().unwrap_or(&EMPTY_ABI);
        let object = ContractArtifactForHash { abi, program: &self.program };

        let serialized = to_string_pythonic(&object).unwrap();
        let serialized_bytes = serialized.as_bytes();

        starknet_keccak(serialized_bytes)
    }
}

/// Serializes legacy ABI entries in "raw" format (without the `type` field).
///
/// The "raw" format is used by starknet-rs and differs from the starknet_api format
/// in that it doesn't include the `type` discriminator field in each ABI entry.
fn serialize_abi_for_hinted_hash<S: Serializer>(
    source: &Vec<LegacyContractClassAbiEntry>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    use starknet_api::deprecated_contract_class::{
        ContractClassAbiEntry, EventAbiEntry, StructAbiEntry, TypedParameter,
    };

    let entries = source;

    // Raw format structs.

    #[derive(Serialize)]
    struct RawTypedParameter<'a> {
        name: &'a String,
        r#type: &'a String,
    }

    #[derive(Serialize)]
    struct RawStructMember<'a> {
        name: &'a String,
        offset: usize,
        r#type: &'a String,
    }

    #[derive(Serialize)]
    struct RawConstructor<'a> {
        name: &'a String,
        inputs: Vec<RawTypedParameter<'a>>,
        outputs: Vec<RawTypedParameter<'a>>,
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct RawFunction<'a> {
        name: &'a String,
        inputs: Vec<RawTypedParameter<'a>>,
        outputs: Vec<RawTypedParameter<'a>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        state_mutability:
            Option<&'a starknet_api::deprecated_contract_class::FunctionStateMutability>,
    }

    #[derive(Serialize)]
    struct RawStruct<'a> {
        name: &'a String,
        size: usize,
        members: Vec<RawStructMember<'a>>,
    }

    #[derive(Serialize)]
    struct RawL1Handler<'a> {
        name: &'a String,
        inputs: Vec<RawTypedParameter<'a>>,
        outputs: Vec<RawTypedParameter<'a>>,
    }

    #[derive(Serialize)]
    struct RawEvent<'a> {
        data: Vec<RawTypedParameter<'a>>,
        keys: Vec<RawTypedParameter<'a>>,
        name: &'a String,
    }

    fn convert_typed_params(params: &[TypedParameter]) -> Vec<RawTypedParameter<'_>> {
        params.iter().map(|p| RawTypedParameter { name: &p.name, r#type: &p.r#type }).collect()
    }

    let mut seq = serializer.serialize_seq(Some(entries.len()))?;

    for entry in entries {
        match entry {
            ContractClassAbiEntry::Constructor(f) => {
                let inputs = convert_typed_params(&f.inputs);
                let outputs = convert_typed_params(&f.outputs);
                seq.serialize_element(&RawConstructor { inputs, name: &f.name, outputs })?;
            }
            ContractClassAbiEntry::Function(f) => {
                let inputs = convert_typed_params(&f.inputs);
                let outputs = convert_typed_params(&f.outputs);
                seq.serialize_element(&RawFunction {
                    inputs,
                    name: &f.name,
                    outputs,
                    state_mutability: f.state_mutability.as_ref(),
                })?;
            }
            ContractClassAbiEntry::L1Handler(f) => {
                let inputs = convert_typed_params(&f.inputs);
                let outputs = convert_typed_params(&f.outputs);
                seq.serialize_element(&RawL1Handler { inputs, name: &f.name, outputs })?;
            }
            ContractClassAbiEntry::Event(EventAbiEntry { data, keys, name, .. }) => {
                let data = convert_typed_params(data);
                let keys = convert_typed_params(keys);
                seq.serialize_element(&RawEvent { data, keys, name })?;
            }
            ContractClassAbiEntry::Struct(StructAbiEntry { members, name, size, .. }) => {
                let members: Vec<_> = members
                    .iter()
                    .map(|m| RawStructMember { name: &m.name, offset: m.offset, r#type: &m.r#type })
                    .collect();
                seq.serialize_element(&RawStruct { members, name, size: *size })?;
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
        #[serde(skip_serializing_if = "Option::is_none")]
        flow_tracking_data: &'a Option<FlowTrackingData>,
        name: &'a String,
        start_pc: &'a u64,
        end_pc: &'a u64,
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
    source: &LegacyProgram,
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

/// The canonical compiled Sierra contract class type.
pub type CasmContractClass = cairo_lang_starknet_classes::casm_contract_class::CasmContractClass;

/// ABI for Sierra-based classes.
pub type ContractAbi = cairo_lang_starknet_classes::abi::Contract;

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(::serde::Serialize, ::serde::Deserialize), serde(untagged))]
pub enum MaybeInvalidSierraContractAbi {
    Valid(ContractAbi),
    Invalid(String),
}

impl std::fmt::Display for MaybeInvalidSierraContractAbi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MaybeInvalidSierraContractAbi::Valid(abi) => {
                let s = to_string_pythonic(abi).expect("failed to serialize abi");
                write!(f, "{}", s)
            }
            MaybeInvalidSierraContractAbi::Invalid(abi) => write!(f, "{}", abi),
        }
    }
}

impl From<String> for MaybeInvalidSierraContractAbi {
    fn from(value: String) -> Self {
        match serde_json::from_str::<ContractAbi>(&value) {
            Ok(abi) => MaybeInvalidSierraContractAbi::Valid(abi),
            Err(..) => MaybeInvalidSierraContractAbi::Invalid(value),
        }
    }
}

impl From<&str> for MaybeInvalidSierraContractAbi {
    fn from(value: &str) -> Self {
        match serde_json::from_str::<ContractAbi>(value) {
            Ok(abi) => MaybeInvalidSierraContractAbi::Valid(abi),
            Err(..) => MaybeInvalidSierraContractAbi::Invalid(value.to_string()),
        }
    }
}

/// Represents a contract in the Starknet network.
///
/// The canonical contract class (Sierra) type.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(::serde::Serialize, ::serde::Deserialize))]
pub struct SierraContractClass {
    pub sierra_program: Vec<BigUintAsHex>,
    pub sierra_program_debug_info: Option<cairo_lang_sierra::debug_info::DebugInfo>,
    pub contract_class_version: String,
    pub entry_points_by_type: ContractEntryPoints,
    pub abi: Option<MaybeInvalidSierraContractAbi>,
}

impl SierraContractClass {
    /// Computes the hash of the Sierra contract class.
    pub fn hash(&self) -> ClassHash {
        let Self { sierra_program, abi, entry_points_by_type, .. } = self;

        let program: Vec<Felt> = sierra_program.iter().map(|f| f.value.clone().into()).collect();
        let abi: String = abi.as_ref().map(|abi| abi.to_string()).unwrap_or_default();

        compute_sierra_class_hash(&abi, entry_points_by_type, &program)
    }
}

impl From<SierraContractClass> for cairo_lang_starknet_classes::contract_class::ContractClass {
    fn from(value: SierraContractClass) -> Self {
        let abi = value.abi.and_then(|abi| match abi {
            MaybeInvalidSierraContractAbi::Invalid(..) => None,
            MaybeInvalidSierraContractAbi::Valid(abi) => Some(abi),
        });

        cairo_lang_starknet_classes::contract_class::ContractClass {
            abi,
            sierra_program: value.sierra_program,
            entry_points_by_type: value.entry_points_by_type,
            contract_class_version: value.contract_class_version,
            sierra_program_debug_info: value.sierra_program_debug_info,
        }
    }
}

impl From<cairo_lang_starknet_classes::contract_class::ContractClass> for SierraContractClass {
    fn from(value: cairo_lang_starknet_classes::contract_class::ContractClass) -> Self {
        SierraContractClass {
            abi: value.abi.map(MaybeInvalidSierraContractAbi::Valid),
            sierra_program: value.sierra_program,
            entry_points_by_type: value.entry_points_by_type,
            contract_class_version: value.contract_class_version,
            sierra_program_debug_info: value.sierra_program_debug_info,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ContractClassCompilationError {
    #[error(transparent)]
    SierraCompilation(#[from] StarknetSierraCompilationError),
}

// NOTE:
// Ideally, we can implement this enum as an untagged `serde` enum, so that we can deserialize from
// the raw JSON class artifact directly into this (ie
// `serde_json::from_str::<ContractClass>(json)`). But that is not possible due to a limitation with untagged enums derivation (see https://github.com/serde-rs/serde/pull/2781).
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(::serde::Serialize, ::serde::Deserialize))]
pub enum ContractClass {
    Class(SierraContractClass),
    Legacy(LegacyContractClass),
}

impl ContractClass {
    /// Computes the hash of the class.
    pub fn class_hash(&self) -> Result<ClassHash, ComputeClassHashError> {
        match self {
            Self::Class(class) => Ok(class.hash()),
            Self::Legacy(class) => compute_legacy_class_hash(class),
        }
    }

    /// Compiles the contract class.
    pub fn compile(self) -> Result<CompiledClass, ContractClassCompilationError> {
        match self {
            Self::Legacy(class) => Ok(CompiledClass::Legacy(class)),
            Self::Class(class) => {
                let casm = CasmContractClass::from_contract_class(class.into(), true, usize::MAX)?;
                let casm = CompiledClass::Class(casm);
                Ok(casm)
            }
        }
    }

    /// Checks if this contract class is a Cairo 0 legacy class.
    ///
    /// Returns `true` if the contract class is a legacy class, `false` otherwise.
    pub fn is_legacy(&self) -> bool {
        self.as_legacy().is_some()
    }

    pub fn as_legacy(&self) -> Option<&LegacyContractClass> {
        match self {
            Self::Legacy(class) => Some(class),
            _ => None,
        }
    }

    pub fn as_sierra(&self) -> Option<&SierraContractClass> {
        match self {
            Self::Class(class) => Some(class),
            _ => None,
        }
    }

    /// Returns the version of the Sierra program of this class.
    pub fn sierra_version(&self) -> SierraVersion {
        match self {
            Self::Class(class) => {
                // The sierra program is an array of field elements and the first six elements are
                // reserved for the compilers version. The array is structured as follows:
                //
                // ┌──────────────────────────────────────┐
                // │ Idx │ Content                        │
                // ┌──────────────────────────────────────┐
                // │ 0   │ Sierra major version           │
                // │ 1   │ Sierra minor version           │
                // │ 2   │ Sierra patch version           │
                // │ 3   │ CASM compiler major version    │
                // │ 4   │ CASM compiler minor version    │
                // │ 5   │ CASM compiler patch version    │
                // │ 6+  │ Program data                   │
                // └──────────────────────────────────────┘
                //

                let version = version_id_from_serialized_sierra_program(&class.sierra_program)
                    .map(|(sierra_id, _)| sierra_id)
                    .expect("invalid sierra program: failed to get version id from sierra program");

                SierraVersion::new(
                    version.major.try_into().unwrap(),
                    version.minor.try_into().unwrap(),
                    version.patch.try_into().unwrap(),
                )
            }

            Self::Legacy(..) => SierraVersion::DEPRECATED,
        }
    }

    /// Returns the length of the Sierra program.
    pub fn sierra_program_len(&self) -> usize {
        match self {
            Self::Class(class) => class.sierra_program.len(),
            // For cairo 0, the sierra_program_length must be 0.
            Self::Legacy(..) => 0,
        }
    }

    // TODO(kariy): document the actual definition of the ABI length here.
    pub fn abi_len(&self) -> usize {
        match self {
            Self::Class(class) => to_string_pythonic(&class.abi.as_ref()).unwrap().len(),
            Self::Legacy(..) => 0,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct ContractClassFromStrError(serde_json::Error);

#[cfg(feature = "serde")]
impl FromStr for ContractClass {
    type Err = ContractClassFromStrError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        #[derive(::serde::Serialize, ::serde::Deserialize)]
        #[allow(clippy::large_enum_variant)]
        #[serde(untagged)]
        enum ContractClassJson {
            Class(SierraContractClass),
            Legacy(LegacyContractClass),
        }

        let class: ContractClassJson =
            serde_json::from_str(s).map_err(ContractClassFromStrError)?;

        match class {
            ContractClassJson::Class(class) => Ok(Self::Class(class)),
            ContractClassJson::Legacy(class) => Ok(Self::Legacy(class)),
        }
    }
}

/// Compiled version of [`ContractClass`].
///
/// This is the CASM format that can be used for execution. TO learn more about CASM, check out the
/// [Starknet docs].
///
/// [Starknet docs]: https://docs.starknet.io/architecture-and-concepts/smart-contracts/cairo-and-sierra/#why_do_we_need_casm
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Eq, PartialEq, derive_more::From)]
#[cfg_attr(feature = "serde", derive(::serde::Serialize, ::serde::Deserialize), serde(untagged))]
pub enum CompiledClass {
    /// The compiled Sierra contract class ie CASM.
    Class(CasmContractClass),

    /// The compiled legacy contract class.
    ///
    /// This is the same as the uncompiled legacy class because prior to Sierra,
    /// the classes were already in CASM format.
    Legacy(LegacyContractClass),
}

impl CompiledClass {
    /// Computes the hash of the compiled class.
    pub fn class_hash(&self) -> Result<CompiledClassHash, ComputeClassHashError> {
        match self {
            Self::Class(class) => Ok(class.compiled_class_hash()),
            Self::Legacy(class) => Ok(compute_legacy_class_hash(class)?),
        }
    }

    /// Checks if the compiled contract class is a legacy (Cairo 0) class.
    ///
    /// Returns `true` if the compiled contract class is a legacy class, `false` otherwise.
    pub fn is_legacy(&self) -> bool {
        matches!(self, Self::Legacy(_))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ComputeClassHashError {
    #[error(transparent)]
    AbiConversion(#[from] serde_json_pythonic::Error),
}

/// Computes the class hash of a Sierra contract class components.
///
/// Implementation reference: https://github.com/starkware-libs/cairo-lang/blob/v0.14.0/src/starkware/starknet/core/os/contract_class/contract_class.cairo
pub fn compute_sierra_class_hash(
    abi: &str,
    entry_points_by_type: &ContractEntryPoints,
    sierra_program: &[Felt],
) -> Felt {
    let mut hasher = starknet_crypto::PoseidonHasher::new();
    hasher.update(short_string!("CONTRACT_CLASS_V0.1.0"));

    // Hashes entry points
    hasher.update(entrypoints_hash(&entry_points_by_type.external));
    hasher.update(entrypoints_hash(&entry_points_by_type.l1_handler));
    hasher.update(entrypoints_hash(&entry_points_by_type.constructor));
    // Hashes ABI
    hasher.update(starknet_keccak(abi.as_bytes()));
    // Hashes Sierra program
    hasher.update(Poseidon::hash_array(sierra_program));

    normalize_address(hasher.finalize())
}

/// Computes the hash of a legacy contract class.
///
/// This function delegates the computation to the `starknet-rs` library. Don't really care about
/// performance here because it's only for legacy classes, but we should definitely find to improve
/// this without introducing too much complexity.
pub fn compute_legacy_class_hash(
    class: &LegacyContractClass,
) -> Result<Felt, ComputeClassHashError> {
    pub use starknet::core::types::contract::legacy::LegacyContractClass as StarknetRsLegacyContractClass;

    let value = serde_json::to_value(class).unwrap();
    let class = serde_json::from_value::<StarknetRsLegacyContractClass>(value).unwrap();
    let hash = class.class_hash().unwrap();

    Ok(hash)
}

fn entrypoints_hash(entrypoints: &[ContractEntryPoint]) -> Felt {
    let mut hasher = starknet_crypto::PoseidonHasher::new();
    for entry in entrypoints {
        hasher.update(entry.selector.clone().into());
        hasher.update(entry.function_idx.into());
    }
    hasher.finalize()
}

fn legacy_entrypoints_hash(entrypoints: &[LegacyContractEntryPoint]) -> Felt {
    let mut hasher = starknet_crypto::PedersenHasher::new();
    for entry in entrypoints {
        hasher.update(entry.selector.0);
        hasher.update(entry.offset.0.into());
    }
    hasher.finalize()
}

#[cfg(test)]
mod tests {

    use starknet::core::types::contract::legacy::LegacyContractClass as StarknetRsLegacyContractClass;
    use starknet::core::types::contract::SierraClass as StarknetRsSierraContractClass;

    use super::{ContractClass, LegacyContractClass, SierraContractClass};

    #[test]
    fn compute_class_hash() {
        let artifact =
            include_str!("../../contracts/build/katana_account_Account.contract_class.json");

        let class = serde_json::from_str::<SierraContractClass>(artifact).unwrap();
        let actual_hash = ContractClass::Class(class).class_hash().unwrap();

        // Compare it against the hash computed using `starknet-rs` types

        let class = serde_json::from_str::<StarknetRsSierraContractClass>(artifact).unwrap();
        let expected_hash = class.class_hash().unwrap();

        assert_eq!(actual_hash, expected_hash);
    }

    #[test]
    fn compute_legacy_class_hash() {
        let artifact = include_str!("../../contracts/build/legacy/erc20.json");

        let class = serde_json::from_str::<LegacyContractClass>(artifact).unwrap();
        let actual_hash = ContractClass::Legacy(class).class_hash().unwrap();

        // Compare it against the hash computed using `starknet-rs` types

        let class = serde_json::from_str::<StarknetRsLegacyContractClass>(artifact).unwrap();
        let expected_hash = class.class_hash().unwrap();

        assert_eq!(actual_hash, expected_hash);
    }

    #[test]
    #[cfg(feature = "serde")]
    fn contract_class_from_str() {
        use std::str::FromStr;

        /////////////////////////////////////////////////////////////////////////
        // Sierra contract class
        /////////////////////////////////////////////////////////////////////////

        let raw = include_str!("../../contracts/build/katana_account_Account.contract_class.json");
        let class = ContractClass::from_str(raw).unwrap();
        assert!(class.as_sierra().is_some());
        assert!(!class.is_legacy());

        /////////////////////////////////////////////////////////////////////////
        // Legacy contract class
        /////////////////////////////////////////////////////////////////////////

        let raw = include_str!("../../contracts/build/legacy/erc20.json");
        let class = ContractClass::from_str(raw).unwrap();
        assert!(class.as_legacy().is_some());
        assert!(class.is_legacy());
    }
}
