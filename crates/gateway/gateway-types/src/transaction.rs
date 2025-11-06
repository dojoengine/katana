use std::io::Read;
use std::io::Write;
use std::sync::Arc;

use base64::Engine;
use cairo_lang_starknet_classes::contract_class::ContractEntryPoints;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use katana_primitives::class::{ClassHash, CompiledClassHash};
use katana_primitives::contract::Nonce;
use katana_primitives::fee::{
    AllResourceBoundsMapping, L1GasResourceBoundsMapping, ResourceBounds, ResourceBoundsMapping,
    Tip,
};
use katana_primitives::transaction::{
    DeclareTx as PrimitiveDeclareTx, DeclareTxV0 as PrimitiveDeclareTxV0,
    DeclareTxV1 as PrimitiveDeclareTxV1, DeclareTxV2 as PrimitiveDeclareTxV2,
    DeclareTxV3 as PrimitiveDeclareTxV3, DeployAccountTx as PrimitiveDeployAccountTx,
    DeployAccountTxV1 as PrimitiveDeployAccountTxV1,
    DeployAccountTxV3 as PrimitiveDeployAccountTxV3, InvokeTx as PrimitiveInvokeTx,
    InvokeTxV0 as PrimitiveInvokeTxV0, InvokeTxV1 as PrimitiveInvokeTxV1,
    InvokeTxV3 as PrimitiveInvokeTxV3, L1HandlerTx as PrimitiveL1HandlerTx, Tx, TxHash, TxType,
    TxWithHash,
};
use katana_primitives::{ContractAddress, Felt};
use katana_rpc_types::SierraClassAbi;
use serde::de;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize};

/// API response for an INVOKE_FUNCTION transaction
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AddInvokeTransactionResponse {
    pub transaction_hash: TxHash,
    pub code: String, // TRANSACTION_RECEIVED
}

/// API response for a DECLARE transaction
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AddDeclareTransactionResponse {
    pub transaction_hash: TxHash,
    pub class_hash: ClassHash,
    pub code: String, // TRANSACTION_RECEIVED
}

/// API response for a DEPLOY ACCOUNT transaction
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AddDeployAccountTransactionResponse {
    pub transaction_hash: TxHash,
    pub code: String, // TRANSACTION_RECEIVED
}

/// Gateway-specific broadcasted invoke transaction.
///
/// This type uses "INVOKE_FUNCTION" as the type tag for serialization to maintain compatibility
/// with the gateway API.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename = "INVOKE_FUNCTION")]
pub struct BroadcastedInvokeTx {
    pub version: Felt,
    pub sender_address: ContractAddress,
    pub calldata: Vec<Felt>,
    pub signature: Vec<Felt>,
    pub nonce: Felt,
    pub tip: katana_primitives::fee::Tip,
    pub paymaster_data: Vec<Felt>,
    pub account_deployment_data: Vec<Felt>,
    #[serde(
        serialize_with = "serialize_resource_bounds_mapping",
        deserialize_with = "deserialize_resource_bounds_mapping"
    )]
    pub resource_bounds: ResourceBoundsMapping,
    pub nonce_data_availability_mode: DataAvailabilityMode,
    pub fee_data_availability_mode: DataAvailabilityMode,
}

/// Gateway-specific broadcasted declare transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename = "DECLARE")]
pub struct BroadcastedDeclareTx {
    #[serde(rename = "version")]
    pub version: Felt,
    pub sender_address: ContractAddress,
    pub compiled_class_hash: CompiledClassHash,
    pub signature: Vec<Felt>,
    pub nonce: Nonce,
    #[serde(
        serialize_with = "serialize_contract_class",
        deserialize_with = "deserialize_contract_class"
    )]
    pub contract_class: std::sync::Arc<katana_rpc_types::class::SierraClass>,
    pub tip: katana_primitives::fee::Tip,
    pub paymaster_data: Vec<Felt>,
    pub account_deployment_data: Vec<Felt>,
    #[serde(
        serialize_with = "serialize_resource_bounds_mapping",
        deserialize_with = "deserialize_resource_bounds_mapping"
    )]
    pub resource_bounds: ResourceBoundsMapping,
    pub nonce_data_availability_mode: DataAvailabilityMode,
    pub fee_data_availability_mode: DataAvailabilityMode,
}

/// Gateway-specific broadcasted deploy account transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename = "DEPLOY_ACCOUNT")]
pub struct BroadcastedDeployAccountTx {
    #[serde(rename = "version")]
    pub version: Felt,
    pub signature: Vec<Felt>,
    pub nonce: Nonce,
    pub contract_address_salt: Felt,
    pub constructor_calldata: Vec<Felt>,
    pub class_hash: ClassHash,
    pub tip: katana_primitives::fee::Tip,
    pub paymaster_data: Vec<Felt>,
    #[serde(
        serialize_with = "serialize_resource_bounds_mapping",
        deserialize_with = "deserialize_resource_bounds_mapping"
    )]
    pub resource_bounds: ResourceBoundsMapping,
    pub nonce_data_availability_mode: DataAvailabilityMode,
    pub fee_data_availability_mode: DataAvailabilityMode,
}

// Conversions from RPC types to Gateway types
impl From<katana_rpc_types::broadcasted::BroadcastedInvokeTx> for BroadcastedInvokeTx {
    fn from(tx: katana_rpc_types::broadcasted::BroadcastedInvokeTx) -> Self {
        let version = if tx.is_query {
            Felt::THREE + katana_rpc_types::broadcasted::QUERY_VERSION_OFFSET
        } else {
            Felt::THREE
        };

        Self {
            version,
            sender_address: tx.sender_address,
            calldata: tx.calldata,
            signature: tx.signature,
            nonce: tx.nonce,
            tip: tx.tip,
            paymaster_data: tx.paymaster_data,
            account_deployment_data: tx.account_deployment_data,
            resource_bounds: tx.resource_bounds,
            nonce_data_availability_mode: tx.nonce_data_availability_mode.into(),
            fee_data_availability_mode: tx.fee_data_availability_mode.into(),
        }
    }
}

impl From<katana_rpc_types::broadcasted::BroadcastedDeclareTx> for BroadcastedDeclareTx {
    fn from(tx: katana_rpc_types::broadcasted::BroadcastedDeclareTx) -> Self {
        let version = if tx.is_query {
            Felt::THREE + katana_rpc_types::broadcasted::QUERY_VERSION_OFFSET
        } else {
            Felt::THREE
        };

        Self {
            version,
            sender_address: tx.sender_address,
            compiled_class_hash: tx.compiled_class_hash,
            signature: tx.signature,
            nonce: tx.nonce,
            contract_class: tx.contract_class,
            tip: tx.tip,
            paymaster_data: tx.paymaster_data,
            account_deployment_data: tx.account_deployment_data,
            resource_bounds: tx.resource_bounds,
            nonce_data_availability_mode: tx.nonce_data_availability_mode.into(),
            fee_data_availability_mode: tx.fee_data_availability_mode.into(),
        }
    }
}

impl From<katana_rpc_types::broadcasted::BroadcastedDeployAccountTx>
    for BroadcastedDeployAccountTx
{
    fn from(tx: katana_rpc_types::broadcasted::BroadcastedDeployAccountTx) -> Self {
        let version = if tx.is_query {
            Felt::THREE + katana_rpc_types::broadcasted::QUERY_VERSION_OFFSET
        } else {
            Felt::THREE
        };

        Self {
            version,
            signature: tx.signature,
            nonce: tx.nonce,
            contract_address_salt: tx.contract_address_salt,
            constructor_calldata: tx.constructor_calldata,
            class_hash: tx.class_hash,
            tip: tx.tip,
            paymaster_data: tx.paymaster_data,
            resource_bounds: tx.resource_bounds,
            nonce_data_availability_mode: tx.nonce_data_availability_mode.into(),
            fee_data_availability_mode: tx.fee_data_availability_mode.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GatewaySierraClass {
    pub sierra_program: Vec<Felt>,
    pub contract_class_version: String,
    pub entry_points_by_type: ContractEntryPoints,
    pub abi: SierraClassAbi,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfirmedTransaction {
    pub transaction_hash: TxHash,
    #[serde(flatten)]
    pub transaction: TypedTransaction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TypedTransaction {
    #[serde(rename = "DEPLOY")]
    Deploy(DeployTx),

    #[serde(rename = "DECLARE")]
    Declare(DeclareTx),

    #[serde(rename = "L1_HANDLER")]
    L1Handler(L1HandlerTx),

    #[serde(rename = "INVOKE_FUNCTION")]
    InvokeFunction(InvokeTx),

    #[serde(rename = "DEPLOY_ACCOUNT")]
    DeployAccount(DeployAccountTx),
}

/// Invoke transaction enum with version-specific variants
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "version")]
pub enum InvokeTx {
    #[serde(rename = "0x0")]
    V0(InvokeTxV0),

    #[serde(rename = "0x1")]
    V1(InvokeTxV1),

    #[serde(rename = "0x3")]
    V3(InvokeTxV3),
}

pub type InvokeTxV0 = katana_rpc_types::RpcInvokeTxV0;
pub type InvokeTxV1 = katana_rpc_types::RpcInvokeTxV1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvokeTxV3 {
    pub sender_address: ContractAddress,
    pub nonce: Nonce,
    pub calldata: Vec<Felt>,
    pub signature: Vec<Felt>,
    #[serde(
        serialize_with = "serialize_resource_bounds_mapping",
        deserialize_with = "deserialize_resource_bounds_mapping"
    )]
    pub resource_bounds: ResourceBoundsMapping,
    pub tip: Tip,
    pub paymaster_data: Vec<Felt>,
    pub account_deployment_data: Vec<Felt>,
    pub nonce_data_availability_mode: DataAvailabilityMode,
    pub fee_data_availability_mode: DataAvailabilityMode,
}

/// Declare transaction enum with version-specific variants
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "version")]
pub enum DeclareTx {
    #[serde(rename = "0x0")]
    V0(DeclareTxV0),

    #[serde(rename = "0x1")]
    V1(DeclareTxV1),

    #[serde(rename = "0x2")]
    V2(DeclareTxV2),

    #[serde(rename = "0x3")]
    V3(DeclareTxV3),
}

pub type DeclareTxV0 = katana_rpc_types::RpcDeclareTxV0;
pub type DeclareTxV1 = katana_rpc_types::RpcDeclareTxV1;
pub type DeclareTxV2 = katana_rpc_types::RpcDeclareTxV2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeclareTxV3 {
    pub sender_address: ContractAddress,
    pub nonce: Nonce,
    pub signature: Vec<Felt>,
    pub class_hash: ClassHash,
    pub compiled_class_hash: CompiledClassHash,
    #[serde(
        serialize_with = "serialize_resource_bounds_mapping",
        deserialize_with = "deserialize_resource_bounds_mapping"
    )]
    pub resource_bounds: ResourceBoundsMapping,
    pub tip: Tip,
    pub paymaster_data: Vec<Felt>,
    pub account_deployment_data: Vec<Felt>,
    pub nonce_data_availability_mode: DataAvailabilityMode,
    pub fee_data_availability_mode: DataAvailabilityMode,
}

/// Deploy account transaction enum with version-specific variants
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "version")]
pub enum DeployAccountTx {
    #[serde(rename = "0x1")]
    V1(DeployAccountTxV1),

    #[serde(rename = "0x3")]
    V3(DeployAccountTxV3),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployAccountTxV1 {
    pub nonce: Nonce,
    pub signature: Vec<Felt>,
    pub class_hash: ClassHash,
    pub contract_address: Option<ContractAddress>,
    pub contract_address_salt: Felt,
    pub constructor_calldata: Vec<Felt>,
    #[serde(
        serialize_with = "serde_utils::serialize_as_hex",
        deserialize_with = "serde_utils::deserialize_u128"
    )]
    pub max_fee: u128,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployAccountTxV3 {
    pub nonce: Nonce,
    pub signature: Vec<Felt>,
    pub class_hash: ClassHash,
    pub contract_address: Option<ContractAddress>,
    pub contract_address_salt: Felt,
    pub constructor_calldata: Vec<Felt>,
    #[serde(
        serialize_with = "serialize_resource_bounds_mapping",
        deserialize_with = "deserialize_resource_bounds_mapping"
    )]
    pub resource_bounds: ResourceBoundsMapping,
    pub tip: Tip,
    pub paymaster_data: Vec<Felt>,
    pub nonce_data_availability_mode: DataAvailabilityMode,
    pub fee_data_availability_mode: DataAvailabilityMode,
}

/// L1 handler transaction
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct L1HandlerTx {
    /// The L1 to L2 message nonce.
    // rpc and primitive doesn't use optinoal for this field. hence why we can't use rpc or
    // primitives transaction types for l1 handler
    pub nonce: Option<Nonce>,
    /// Transaction version.
    pub version: Felt,
    /// The input to the L1 handler function.
    pub calldata: Vec<Felt>,
    /// Contract address of the L1 handler.
    pub contract_address: ContractAddress,
    /// The L1 handler function selector.
    pub entry_point_selector: Felt,
}

/// Deploy transaction - legacy transaction type
pub type DeployTx = katana_primitives::transaction::DeployTx;

// This is one of the main reason why have to redundantly redefine some of the transaction types
// instead of using the rpc variations. And the reason why don't just update the serde
// implementation of `DataAvailabilityMode` in katana-primitives (1) to avoid breaking the database
// format (though this is not that serious as we can implement a serde impl that works with both
// formats), (2) bcs this type is also used in the rpc types and adding another serialization format
// will mean that we can't guarantee rpc format invariant.
//
// We redundantly define the `DataAvailabilityMode` enum here because the serde implementation is
// different from the one in the `katana_primitives` crate. And changing the serde implementation in
// the `katana_primitives` crate would break the database format. So, we have to define the type
// again. But see if we can remove it once we're okay with breaking the database format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataAvailabilityMode {
    L1,
    L2,
}

impl Serialize for DataAvailabilityMode {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let value = match self {
            DataAvailabilityMode::L1 => 0u8,
            DataAvailabilityMode::L2 => 1u8,
        };
        value.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for DataAvailabilityMode {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = u8::deserialize(deserializer)?;
        match value {
            0 => Ok(DataAvailabilityMode::L1),
            1 => Ok(DataAvailabilityMode::L2),
            _ => Err(serde::de::Error::custom(format!(
                "Invalid data availability mode; expected 0 or 1 but got {value}"
            ))),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TxTryFromError {
    #[error("unsupported transaction version; type: {r#type:?}, version: {version:#x}")]
    UnsupportedVersion { r#type: TxType, version: Felt },
}

// -- Conversion to Katana primitive types.

impl TryFrom<ConfirmedTransaction> for TxWithHash {
    type Error = TxTryFromError;

    fn try_from(tx: ConfirmedTransaction) -> Result<Self, Self::Error> {
        let transaction = match tx.transaction {
            TypedTransaction::Deploy(tx) => Tx::Deploy(tx),
            TypedTransaction::Declare(tx) => Tx::Declare(tx.try_into()?),
            TypedTransaction::L1Handler(tx) => Tx::L1Handler(tx.into()),
            TypedTransaction::InvokeFunction(tx) => Tx::Invoke(tx.try_into()?),
            TypedTransaction::DeployAccount(tx) => Tx::DeployAccount(tx.try_into()?),
        };

        Ok(TxWithHash { hash: tx.transaction_hash, transaction })
    }
}

impl TryFrom<InvokeTx> for PrimitiveInvokeTx {
    type Error = TxTryFromError;

    fn try_from(value: InvokeTx) -> Result<Self, Self::Error> {
        match value {
            InvokeTx::V0(tx) => Ok(PrimitiveInvokeTx::V0(PrimitiveInvokeTxV0 {
                calldata: tx.calldata,
                signature: tx.signature,
                contract_address: tx.contract_address,
                max_fee: tx.max_fee,
                entry_point_selector: tx.entry_point_selector,
            })),
            InvokeTx::V1(tx) => Ok(PrimitiveInvokeTx::V1(PrimitiveInvokeTxV1 {
                chain_id: Default::default(),
                nonce: tx.nonce,
                calldata: tx.calldata,
                signature: tx.signature,
                max_fee: tx.max_fee,
                sender_address: tx.sender_address,
            })),
            InvokeTx::V3(tx) => Ok(PrimitiveInvokeTx::V3(PrimitiveInvokeTxV3 {
                tip: tx.tip.into(),
                paymaster_data: tx.paymaster_data,
                chain_id: Default::default(),
                nonce: tx.nonce,
                calldata: tx.calldata,
                signature: tx.signature,
                sender_address: tx.sender_address,
                resource_bounds: tx.resource_bounds,
                account_deployment_data: tx.account_deployment_data,
                fee_data_availability_mode: tx.fee_data_availability_mode.into(),
                nonce_data_availability_mode: tx.nonce_data_availability_mode.into(),
            })),
        }
    }
}

impl TryFrom<DeclareTx> for PrimitiveDeclareTx {
    type Error = TxTryFromError;

    fn try_from(value: DeclareTx) -> Result<Self, Self::Error> {
        match value {
            DeclareTx::V0(tx) => Ok(PrimitiveDeclareTx::V0(PrimitiveDeclareTxV0 {
                signature: tx.signature,
                chain_id: Default::default(),
                class_hash: tx.class_hash,
                sender_address: tx.sender_address,
                max_fee: tx.max_fee,
            })),
            DeclareTx::V1(tx) => Ok(PrimitiveDeclareTx::V1(PrimitiveDeclareTxV1 {
                chain_id: Default::default(),
                sender_address: tx.sender_address,
                nonce: tx.nonce,
                signature: tx.signature,
                class_hash: tx.class_hash,
                max_fee: tx.max_fee,
            })),
            DeclareTx::V2(tx) => Ok(PrimitiveDeclareTx::V2(PrimitiveDeclareTxV2 {
                chain_id: Default::default(),
                sender_address: tx.sender_address,
                nonce: tx.nonce,
                signature: tx.signature,
                class_hash: tx.class_hash,
                compiled_class_hash: tx.compiled_class_hash,
                max_fee: tx.max_fee,
            })),
            DeclareTx::V3(tx) => Ok(PrimitiveDeclareTx::V3(PrimitiveDeclareTxV3 {
                chain_id: Default::default(),
                sender_address: tx.sender_address,
                nonce: tx.nonce,
                signature: tx.signature,
                class_hash: tx.class_hash,
                compiled_class_hash: tx.compiled_class_hash,
                resource_bounds: tx.resource_bounds,
                tip: tx.tip.into(),
                paymaster_data: tx.paymaster_data,
                account_deployment_data: tx.account_deployment_data,
                nonce_data_availability_mode: tx.nonce_data_availability_mode.into(),
                fee_data_availability_mode: tx.fee_data_availability_mode.into(),
            })),
        }
    }
}

impl TryFrom<DeployAccountTx> for PrimitiveDeployAccountTx {
    type Error = TxTryFromError;

    fn try_from(value: DeployAccountTx) -> Result<Self, Self::Error> {
        match value {
            DeployAccountTx::V1(tx) => {
                Ok(PrimitiveDeployAccountTx::V1(PrimitiveDeployAccountTxV1 {
                    chain_id: Default::default(),
                    nonce: tx.nonce,
                    signature: tx.signature,
                    class_hash: tx.class_hash,
                    contract_address: tx.contract_address.unwrap_or_default(),
                    contract_address_salt: tx.contract_address_salt,
                    constructor_calldata: tx.constructor_calldata,
                    max_fee: tx.max_fee,
                }))
            }
            DeployAccountTx::V3(tx) => {
                Ok(PrimitiveDeployAccountTx::V3(PrimitiveDeployAccountTxV3 {
                    chain_id: Default::default(),
                    nonce: tx.nonce,
                    signature: tx.signature,
                    class_hash: tx.class_hash,
                    contract_address: tx.contract_address.unwrap_or_default(),
                    contract_address_salt: tx.contract_address_salt,
                    constructor_calldata: tx.constructor_calldata,
                    resource_bounds: tx.resource_bounds,
                    tip: tx.tip.into(),
                    paymaster_data: tx.paymaster_data,
                    nonce_data_availability_mode: tx.nonce_data_availability_mode.into(),
                    fee_data_availability_mode: tx.fee_data_availability_mode.into(),
                }))
            }
        }
    }
}

impl From<L1HandlerTx> for PrimitiveL1HandlerTx {
    fn from(value: L1HandlerTx) -> Self {
        Self {
            version: value.version,
            calldata: value.calldata,
            chain_id: Default::default(),
            message_hash: Default::default(),
            paid_fee_on_l1: Default::default(),
            nonce: value.nonce.unwrap_or_default(),
            contract_address: value.contract_address,
            entry_point_selector: value.entry_point_selector,
        }
    }
}

impl From<DataAvailabilityMode> for katana_primitives::da::DataAvailabilityMode {
    fn from(mode: DataAvailabilityMode) -> Self {
        match mode {
            DataAvailabilityMode::L1 => Self::L1,
            DataAvailabilityMode::L2 => Self::L2,
        }
    }
}

// Custom serialization for contract class with gzip + base64 encoded sierra program
fn serialize_contract_class<S: serde::Serializer>(
    class: &Arc<katana_rpc_types::class::SierraClass>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let mut state = serializer.serialize_struct("GatewaySierraClass", 4)?;

    // Convert sierra_program (Vec<Felt>) to JSON array, then gzip compress, then base64 encode
    let program_json =
        serde_json::to_string(&class.sierra_program).map_err(serde::ser::Error::custom)?;

    // Gzip compress the JSON
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(program_json.as_bytes())
        .map_err(|e| serde::ser::Error::custom(format!("gzip compression failed: {e}")))?;
    let compressed_bytes = encoder
        .finish()
        .map_err(|e| serde::ser::Error::custom(format!("gzip finish failed: {e}")))?;

    // Base64 encode
    let program_base64 = base64::engine::general_purpose::STANDARD.encode(&compressed_bytes);

    state.serialize_field("sierra_program", &program_base64)?;
    state.serialize_field("contract_class_version", &class.contract_class_version)?;
    state.serialize_field("entry_points_by_type", &class.entry_points_by_type)?;

    // Serialize ABI - it's already in pythonic JSON format via SierraClassAbi's Serialize impl
    let abi_str = class.abi.to_string();
    state.serialize_field("abi", &abi_str)?;

    state.end()
}

fn deserialize_contract_class<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Arc<katana_rpc_types::class::SierraClass>, D::Error> {
    #[derive(Deserialize)]
    struct GatewaySierraClass {
        sierra_program: String,
        contract_class_version: String,
        entry_points_by_type: cairo_lang_starknet_classes::contract_class::ContractEntryPoints,
        abi: String,
    }

    let gateway_class = GatewaySierraClass::deserialize(deserializer)?;

    // Base64 decode
    let compressed_bytes = base64::engine::general_purpose::STANDARD
        .decode(&gateway_class.sierra_program)
        .map_err(|e| de::Error::custom(format!("failed to decode base64 sierra_program: {e}")))?;

    // Gzip decompress
    let mut decoder = GzDecoder::new(&compressed_bytes[..]);
    let mut decompressed = String::new();
    decoder
        .read_to_string(&mut decompressed)
        .map_err(|e| de::Error::custom(format!("failed to decompress sierra_program: {e}")))?;

    // Parse JSON array to Vec<Felt>
    let sierra_program: Vec<Felt> = serde_json::from_str(&decompressed)
        .map_err(|e| de::Error::custom(format!("failed to parse sierra_program JSON: {e}")))?;

    // Deserialize ABI from pythonic JSON string
    let abi: katana_rpc_types::class::SierraClassAbi =
        if gateway_class.abi.is_empty() || gateway_class.abi == "[]" {
            Default::default()
        } else {
            serde_json::from_str(&gateway_class.abi)
                .map_err(|e| de::Error::custom(format!("invalid abi: {e}")))?
        };

    Ok(std::sync::Arc::new(katana_rpc_types::class::SierraClass {
        sierra_program,
        contract_class_version: gateway_class.contract_class_version,
        entry_points_by_type: gateway_class.entry_points_by_type,
        abi,
    }))
}

fn deserialize_resource_bounds_mapping<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<ResourceBoundsMapping, D::Error> {
    #[derive(Deserialize)]
    struct FeederGatewayResourceBounds {
        #[serde(rename = "L1_GAS")]
        l1_gas: ResourceBounds,
        #[serde(rename = "L2_GAS")]
        l2_gas: ResourceBounds,
        #[serde(rename = "L1_DATA_GAS")]
        l1_data_gas: Option<ResourceBounds>,
    }

    let bounds = FeederGatewayResourceBounds::deserialize(deserializer)?;

    if let Some(l1_data_gas) = bounds.l1_data_gas {
        Ok(ResourceBoundsMapping::All(AllResourceBoundsMapping {
            l1_gas: bounds.l1_gas,
            l2_gas: bounds.l2_gas,
            l1_data_gas,
        }))
    } else {
        Ok(ResourceBoundsMapping::L1Gas(L1GasResourceBoundsMapping {
            l1_gas: bounds.l1_gas,
            l2_gas: bounds.l2_gas,
        }))
    }
}

fn serialize_resource_bounds_mapping<S: serde::Serializer>(
    bounds: &ResourceBoundsMapping,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    #[derive(Serialize)]
    struct FeederGatewayResourceBounds<'a> {
        #[serde(rename = "L1_GAS")]
        l1_gas: &'a ResourceBounds,
        #[serde(rename = "L2_GAS")]
        l2_gas: &'a ResourceBounds,
        #[serde(rename = "L1_DATA_GAS")]
        l1_data_gas: Option<&'a ResourceBounds>,
    }

    let feeder_bounds = match bounds {
        ResourceBoundsMapping::All(all_bounds) => FeederGatewayResourceBounds {
            l1_gas: &all_bounds.l1_gas,
            l2_gas: &all_bounds.l2_gas,
            l1_data_gas: Some(&all_bounds.l1_data_gas),
        },
        ResourceBoundsMapping::L1Gas(l1_gas_bounds) => FeederGatewayResourceBounds {
            l1_gas: &l1_gas_bounds.l1_gas,
            l2_gas: &l1_gas_bounds.l2_gas,
            l1_data_gas: None,
        },
    };

    feeder_bounds.serialize(serializer)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use katana_primitives::fee::{ResourceBounds, ResourceBoundsMapping, Tip};
    use katana_primitives::Felt;
    use serde_json::json;

    use super::*;

    #[test]
    fn test_gateway_invoke_tx_serialization() {
        let gateway_tx = BroadcastedInvokeTx {
            version: Felt::THREE,
            sender_address: ContractAddress::from(Felt::from(0x123)),
            calldata: vec![Felt::ONE, Felt::TWO],
            signature: vec![],
            nonce: Felt::ZERO,
            tip: Tip::from(0),
            paymaster_data: vec![],
            account_deployment_data: vec![],
            resource_bounds: ResourceBoundsMapping::L1Gas(
                katana_primitives::fee::L1GasResourceBoundsMapping {
                    l1_gas: ResourceBounds { max_amount: 100, max_price_per_unit: 200 },
                    l2_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
                },
            ),
            fee_data_availability_mode: katana_primitives::da::DataAvailabilityMode::L1.into(),
            nonce_data_availability_mode: katana_primitives::da::DataAvailabilityMode::L1.into(),
        };

        let serialized = serde_json::to_value(&gateway_tx).unwrap();

        // Verify that the type field is "INVOKE_FUNCTION"
        assert_eq!(serialized["type"], "INVOKE_FUNCTION");
        assert_eq!(serialized["version"], "0x3");
        assert_eq!(serialized["sender_address"], "0x123");
    }

    #[test]
    fn test_gateway_invoke_tx_deserialization() {
        let json = json!({
            "type": "INVOKE_FUNCTION",
            "version": "0x3",
            "sender_address": "0x123",
            "calldata": ["0x1", "0x2"],
            "signature": [],
            "nonce": "0x0",
            "paymaster_data": [],
            "tip": "0x0",
            "account_deployment_data": [],
            "resource_bounds": {
                "L1_GAS": {
                    "max_amount": "0x100",
                    "max_price_per_unit": "0x200"
                },
                "L2_GAS": {
                    "max_amount": "0x0",
                    "max_price_per_unit": "0x0"
                }
            },
            "fee_data_availability_mode": 0,
            "nonce_data_availability_mode": 0
        });

        let gateway_tx: BroadcastedInvokeTx = serde_json::from_value(json).unwrap();
        assert_eq!(gateway_tx.version, Felt::THREE);
        assert_eq!(gateway_tx.sender_address, ContractAddress::from(Felt::from(0x123)));
        assert_eq!(gateway_tx.calldata, vec![Felt::ONE, Felt::TWO]);
    }

    #[test]
    fn test_gateway_declare_tx_serialization() {
        let sierra_class = Arc::new(katana_rpc_types::class::SierraClass {
            sierra_program: vec![Felt::ONE, Felt::TWO],
            contract_class_version: "0.1.0".to_string(),
            entry_points_by_type: Default::default(),
            abi: Default::default(),
        });

        let gateway_tx = BroadcastedDeclareTx {
            version: Felt::THREE,
            sender_address: ContractAddress::from(Felt::from(0x456)),
            compiled_class_hash: Felt::from(0x789),
            signature: vec![],
            nonce: Felt::ONE,
            contract_class: sierra_class,
            tip: Tip::from(0),
            paymaster_data: vec![],
            account_deployment_data: vec![],
            resource_bounds: ResourceBoundsMapping::L1Gas(
                katana_primitives::fee::L1GasResourceBoundsMapping {
                    l1_gas: ResourceBounds { max_amount: 100, max_price_per_unit: 200 },
                    l2_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
                },
            ),
            fee_data_availability_mode: katana_primitives::da::DataAvailabilityMode::L1.into(),
            nonce_data_availability_mode: katana_primitives::da::DataAvailabilityMode::L1.into(),
        };

        let serialized = serde_json::to_value(&gateway_tx).unwrap();

        // Verify that the type field is "DECLARE"
        assert_eq!(serialized["type"], "DECLARE");
        assert_eq!(serialized["version"], "0x3");
    }

    #[test]
    fn test_gateway_deploy_account_tx_serialization() {
        let gateway_tx = BroadcastedDeployAccountTx {
            version: Felt::THREE,
            signature: vec![Felt::ONE, Felt::TWO],
            nonce: Felt::ZERO,
            contract_address_salt: Felt::from(0xabc),
            constructor_calldata: vec![Felt::ONE, Felt::TWO],
            class_hash: Felt::from(0xdef),
            tip: Tip::from(0),
            paymaster_data: vec![],
            resource_bounds: ResourceBoundsMapping::L1Gas(
                katana_primitives::fee::L1GasResourceBoundsMapping {
                    l1_gas: ResourceBounds { max_amount: 100, max_price_per_unit: 200 },
                    l2_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
                },
            ),
            fee_data_availability_mode: katana_primitives::da::DataAvailabilityMode::L1.into(),
            nonce_data_availability_mode: katana_primitives::da::DataAvailabilityMode::L1.into(),
        };

        let serialized = serde_json::to_value(&gateway_tx).unwrap();

        // Verify that the type field is "DEPLOY_ACCOUNT"
        assert_eq!(serialized["type"], "DEPLOY_ACCOUNT");
        assert_eq!(serialized["version"], "0x3");
    }

    #[test]
    fn test_conversion_from_rpc_invoke_tx() {
        let rpc_tx = katana_rpc_types::broadcasted::BroadcastedInvokeTx {
            sender_address: ContractAddress::from(Felt::from(0x123)),
            calldata: vec![Felt::ONE, Felt::TWO],
            signature: vec![],
            nonce: Felt::ZERO,
            paymaster_data: vec![],
            tip: Tip::from(0),
            account_deployment_data: vec![],
            resource_bounds: ResourceBoundsMapping::L1Gas(
                katana_primitives::fee::L1GasResourceBoundsMapping {
                    l1_gas: ResourceBounds { max_amount: 100, max_price_per_unit: 200 },
                    l2_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
                },
            ),
            fee_data_availability_mode: katana_primitives::da::DataAvailabilityMode::L1,
            nonce_data_availability_mode: katana_primitives::da::DataAvailabilityMode::L1,
            is_query: false,
        };

        let gateway_tx: BroadcastedInvokeTx = rpc_tx.into();
        assert_eq!(gateway_tx.version, Felt::THREE);
        assert_eq!(gateway_tx.sender_address, ContractAddress::from(Felt::from(0x123)));

        // Test serialization produces correct type tag
        let serialized = serde_json::to_value(&gateway_tx).unwrap();
        assert_eq!(serialized["type"], "INVOKE_FUNCTION");
    }

    #[test]
    fn test_conversion_from_rpc_query_invoke_tx() {
        let rpc_tx = katana_rpc_types::broadcasted::BroadcastedInvokeTx {
            sender_address: ContractAddress::from(Felt::from(0x123)),
            calldata: vec![Felt::ONE, Felt::TWO],
            signature: vec![],
            nonce: Felt::ZERO,
            paymaster_data: vec![],
            tip: Tip::from(0),
            account_deployment_data: vec![],
            resource_bounds: ResourceBoundsMapping::L1Gas(
                katana_primitives::fee::L1GasResourceBoundsMapping {
                    l1_gas: ResourceBounds { max_amount: 100, max_price_per_unit: 200 },
                    l2_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
                },
            ),
            fee_data_availability_mode: katana_primitives::da::DataAvailabilityMode::L1,
            nonce_data_availability_mode: katana_primitives::da::DataAvailabilityMode::L1,
            is_query: true,
        };

        let gateway_tx: BroadcastedInvokeTx = rpc_tx.into();
        // Query version should have offset added
        assert_eq!(
            gateway_tx.version,
            Felt::THREE + katana_rpc_types::broadcasted::QUERY_VERSION_OFFSET
        );
    }

    #[test]
    fn test_sierra_program_base64_encoding() {
        let sierra_class = Arc::new(katana_rpc_types::class::SierraClass {
            sierra_program: vec![Felt::from(0x123), Felt::from(0x456)],
            contract_class_version: "0.1.0".to_string(),
            entry_points_by_type: Default::default(),
            abi: Default::default(),
        });

        let gateway_tx = BroadcastedDeclareTx {
            version: Felt::THREE,
            sender_address: ContractAddress::from(Felt::from(0x789)),
            compiled_class_hash: Felt::from(0xabc),
            signature: vec![],
            nonce: Felt::ONE,
            contract_class: sierra_class.clone(),
            tip: Tip::from(0),
            paymaster_data: vec![],
            account_deployment_data: vec![],
            resource_bounds: ResourceBoundsMapping::L1Gas(
                katana_primitives::fee::L1GasResourceBoundsMapping {
                    l1_gas: ResourceBounds { max_amount: 100, max_price_per_unit: 200 },
                    l2_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
                },
            ),
            fee_data_availability_mode: katana_primitives::da::DataAvailabilityMode::L1.into(),
            nonce_data_availability_mode: katana_primitives::da::DataAvailabilityMode::L1.into(),
        };

        let serialized = serde_json::to_value(&gateway_tx).unwrap();

        // Verify sierra_program is a base64 string, not an array
        assert!(serialized["contract_class"]["sierra_program"].is_string());

        // Verify we can deserialize it back
        let deserialized: BroadcastedDeclareTx = serde_json::from_value(serialized).unwrap();
        assert_eq!(deserialized.contract_class.sierra_program, sierra_class.sierra_program);
    }
}
