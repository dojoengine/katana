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
use serde::{Deserialize, Deserializer, Serialize};

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
