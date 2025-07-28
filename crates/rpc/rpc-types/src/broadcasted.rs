use std::sync::Arc;

use katana_primitives::chain::ChainId;
use katana_primitives::class::{
    ClassHash, CompiledClassHash, ComputeClassHashError, ContractClass, SierraContractClass,
};
use katana_primitives::contract::Nonce;
use katana_primitives::da::DataAvailabilityMode;
use katana_primitives::fee::{AllResourceBoundsMapping, ResourceBounds, ResourceBoundsMapping};
use katana_primitives::transaction::{
    DeclareTx, DeclareTxV3, DeclareTxWithClass, DeployAccountTx, DeployAccountTxV3, InvokeTx,
    InvokeTxV3, TxHash, TxType,
};
use katana_primitives::utils::serde::{
    deserialize_hex_u128, deserialize_hex_u64, serialize_hex_u128, serialize_hex_u64,
};
use katana_primitives::{ContractAddress, Felt};
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use starknet::core::utils::get_contract_address;

use crate::class::RpcSierraContractClass;

const QUERY_VERSION_OFFSET: Felt =
    Felt::from_raw([576460752142434320, 18446744073709551584, 17407, 18446744073700081665]);

macro_rules! expect_field {
    ($field:expr, $tx_type:expr, $field_name:literal) => {
        $field.ok_or(UntypedBroadcastedTxError::MissingField {
            r#type: $tx_type,
            field: $field_name,
        })?
    };
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UntypedBroadcastedTxError {
    #[error("expected {expected} transaction, got {actual}")]
    InvalidTxType { expected: TxType, actual: TxType },

    #[error("missing `{field}` for {r#type} transaction")]
    MissingField { r#type: TxType, field: &'static str },

    #[error("invalid version {version} for {r#type} transaction")]
    InvalidVersion { version: Felt, r#type: TxType },

    #[error("unsupported transaction type {r#type}")]
    UnsupportedTxType { r#type: TxType },
}

#[serde_with::serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UntypedBroadcastedTx {
    // Common fields
    pub r#type: TxType,
    pub version: Felt,
    pub nonce: Felt,
    pub signature: Vec<Felt>,
    #[serde(serialize_with = "serialize_hex_u64", deserialize_with = "deserialize_hex_u64")]
    pub tip: u64,
    pub paymaster_data: Vec<Felt>,
    #[serde(
        serialize_with = "serialize_resource_bounds_mapping",
        deserialize_with = "deserialize_resource_bounds_mapping"
    )]
    pub resource_bounds: ResourceBoundsMapping,
    pub nonce_data_availability_mode: DataAvailabilityMode,
    pub fee_data_availability_mode: DataAvailabilityMode,

    // Invoke only fields
    #[serde(default)]
    pub calldata: Option<Vec<Felt>>,

    // Declare only fields
    #[serde(default)]
    pub compiled_class_hash: Option<CompiledClassHash>,
    #[serde(default)]
    pub contract_class: Option<Arc<RpcSierraContractClass>>,

    // Invoke & Declare only field
    #[serde(default)]
    pub sender_address: Option<ContractAddress>,
    #[serde(default)]
    pub account_deployment_data: Option<Vec<Felt>>,

    // DeployAccount only fields
    #[serde(default)]
    pub contract_address_salt: Option<Felt>,
    #[serde(default)]
    pub constructor_calldata: Option<Vec<Felt>>,
    #[serde(default)]
    pub class_hash: Option<ClassHash>,
}

impl UntypedBroadcastedTx {
    /// Convert into a typed transaction.
    pub fn typed(self) -> Result<BroadcastedTx, UntypedBroadcastedTxError> {
        match self.r#type {
            TxType::Invoke => Ok(BroadcastedTx::Invoke(self.try_into_invoke()?)),
            TxType::Declare => Ok(BroadcastedTx::Declare(self.try_into_declare()?)),
            TxType::DeployAccount => {
                Ok(BroadcastedTx::DeployAccount(self.try_into_deploy_account()?))
            }
            r#type => Err(UntypedBroadcastedTxError::UnsupportedTxType { r#type }),
        }
    }

    /// Convert into an **Invoke** transaction.
    pub fn try_into_invoke(self) -> Result<BroadcastedInvokeTx, UntypedBroadcastedTxError> {
        if self.r#type != TxType::Invoke {
            return Err(UntypedBroadcastedTxError::InvalidTxType {
                expected: TxType::Invoke,
                actual: self.r#type,
            });
        }

        let is_query = if self.version == Felt::THREE {
            false
        } else if self.version == Felt::THREE + QUERY_VERSION_OFFSET {
            true
        } else {
            return Err(UntypedBroadcastedTxError::InvalidVersion {
                version: self.version,
                r#type: TxType::Invoke,
            });
        };

        let calldata = expect_field!(self.calldata, TxType::Invoke, "calldata");
        let sender_address = expect_field!(self.sender_address, TxType::Invoke, "sender_address");
        let account_deployment_data =
            expect_field!(self.account_deployment_data, TxType::Invoke, "account_deployment_data");

        Ok(BroadcastedInvokeTx {
            calldata,
            tip: self.tip,
            sender_address,
            nonce: self.nonce,
            account_deployment_data,
            signature: self.signature,
            paymaster_data: self.paymaster_data,
            resource_bounds: self.resource_bounds,
            fee_data_availability_mode: self.fee_data_availability_mode,
            nonce_data_availability_mode: self.nonce_data_availability_mode,
            is_query,
        })
    }

    /// Convert into a **Declare** transaction.
    pub fn try_into_declare(self) -> Result<BroadcastedDeclareTx, UntypedBroadcastedTxError> {
        if self.r#type != TxType::Declare {
            return Err(UntypedBroadcastedTxError::InvalidTxType {
                expected: TxType::Declare,
                actual: self.r#type,
            });
        }

        let is_query = if self.version == Felt::THREE {
            false
        } else if self.version == Felt::THREE + QUERY_VERSION_OFFSET {
            true
        } else {
            return Err(UntypedBroadcastedTxError::InvalidVersion {
                version: self.version,
                r#type: TxType::Declare,
            });
        };

        let sender_address = expect_field!(self.sender_address, TxType::Declare, "sender_address");
        let compiled_class_hash =
            expect_field!(self.compiled_class_hash, TxType::Declare, "compiled_class_hash");
        let contract_class = expect_field!(self.contract_class, TxType::Declare, "contract_class");
        let account_deployment_data =
            expect_field!(self.account_deployment_data, TxType::Declare, "account_deployment_data");

        Ok(BroadcastedDeclareTx {
            contract_class,
            sender_address,
            tip: self.tip,
            nonce: self.nonce,
            compiled_class_hash,
            account_deployment_data,
            signature: self.signature,
            paymaster_data: self.paymaster_data,
            resource_bounds: self.resource_bounds,
            nonce_data_availability_mode: self.nonce_data_availability_mode,
            fee_data_availability_mode: self.fee_data_availability_mode,
            is_query,
        })
    }

    /// Convert into a **DeployAccount** transaction.
    pub fn try_into_deploy_account(
        self,
    ) -> Result<BroadcastedDeployAccountTx, UntypedBroadcastedTxError> {
        if self.r#type != TxType::DeployAccount {
            return Err(UntypedBroadcastedTxError::InvalidTxType {
                expected: TxType::DeployAccount,
                actual: self.r#type,
            });
        }

        let is_query = if self.version == Felt::THREE {
            false
        } else if self.version == Felt::THREE + QUERY_VERSION_OFFSET {
            true
        } else {
            return Err(UntypedBroadcastedTxError::InvalidVersion {
                version: self.version,
                r#type: TxType::DeployAccount,
            });
        };

        let contract_address_salt = expect_field!(
            self.contract_address_salt,
            TxType::DeployAccount,
            "contract_address_salt"
        );
        let constructor_calldata =
            expect_field!(self.constructor_calldata, TxType::DeployAccount, "constructor_calldata");
        let class_hash = expect_field!(self.class_hash, TxType::DeployAccount, "class_hash");

        Ok(BroadcastedDeployAccountTx {
            class_hash,
            tip: self.tip,
            nonce: self.nonce,
            constructor_calldata,
            contract_address_salt,
            signature: self.signature,
            paymaster_data: self.paymaster_data,
            resource_bounds: self.resource_bounds,
            fee_data_availability_mode: self.fee_data_availability_mode,
            nonce_data_availability_mode: self.nonce_data_availability_mode,
            is_query,
        })
    }
}

#[derive(Debug, Clone)]
pub enum BroadcastedTx {
    Invoke(BroadcastedInvokeTx),
    Declare(BroadcastedDeclareTx),
    DeployAccount(BroadcastedDeployAccountTx),
}

impl Serialize for BroadcastedTx {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            BroadcastedTx::Invoke(tx) => tx.serialize(serializer),
            BroadcastedTx::Declare(tx) => tx.serialize(serializer),
            BroadcastedTx::DeployAccount(tx) => tx.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for BroadcastedTx {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        UntypedBroadcastedTx::deserialize(deserializer)?.typed().map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone)]
pub struct BroadcastedInvokeTx {
    pub sender_address: ContractAddress,
    pub calldata: Vec<Felt>,
    pub signature: Vec<Felt>,
    pub nonce: Felt,
    pub paymaster_data: Vec<Felt>,
    pub tip: u64,
    pub account_deployment_data: Vec<Felt>,
    pub resource_bounds: ResourceBoundsMapping,
    pub fee_data_availability_mode: DataAvailabilityMode,
    pub nonce_data_availability_mode: DataAvailabilityMode,
    pub is_query: bool,
}

impl BroadcastedInvokeTx {
    pub fn is_query(&self) -> bool {
        self.is_query
    }

    pub fn into_inner(self, chain_id: ChainId) -> InvokeTx {
        InvokeTx::V3(InvokeTxV3 {
            chain_id,
            tip: self.tip,
            nonce: self.nonce,
            calldata: self.calldata,
            signature: self.signature,
            sender_address: self.sender_address,
            paymaster_data: self.paymaster_data,
            resource_bounds: self.resource_bounds,
            account_deployment_data: self.account_deployment_data,
            fee_data_availability_mode: self.fee_data_availability_mode,
            nonce_data_availability_mode: self.nonce_data_availability_mode,
        })
    }
}

impl serde::Serialize for BroadcastedInvokeTx {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;

        let version = if self.is_query { Felt::THREE + QUERY_VERSION_OFFSET } else { Felt::THREE };

        let mut state = serializer.serialize_struct("BroadcastedInvokeTx", 11)?;
        state.serialize_field("type", "INVOKE")?;
        state.serialize_field("version", &version)?;
        state.serialize_field("sender_address", &self.sender_address)?;
        state.serialize_field("calldata", &self.calldata)?;
        state.serialize_field("signature", &self.signature)?;
        state.serialize_field("nonce", &self.nonce)?;
        state.serialize_field("paymaster_data", &self.paymaster_data)?;
        state.serialize_field("tip", &self.tip)?;
        state.serialize_field("account_deployment_data", &self.account_deployment_data)?;
        state.serialize_field("resource_bounds", &self.resource_bounds)?;
        state.serialize_field("fee_data_availability_mode", &self.fee_data_availability_mode)?;
        state
            .serialize_field("nonce_data_availability_mode", &self.nonce_data_availability_mode)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for BroadcastedInvokeTx {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        UntypedBroadcastedTx::deserialize(deserializer)?
            .try_into_invoke()
            .map_err(de::Error::custom)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BroadcastedDeclareTxError {
    #[error("failed to compute class hash: {0}")]
    ComputeClassHash(#[from] ComputeClassHashError),
    #[error("invalid contract class: {0}")]
    InvalidContractClass(#[from] crate::class::ConversionError),
}

#[derive(Debug, Clone)]
pub struct BroadcastedDeclareTx {
    pub sender_address: ContractAddress,
    pub compiled_class_hash: CompiledClassHash,
    pub signature: Vec<Felt>,
    pub nonce: Nonce,
    pub contract_class: Arc<RpcSierraContractClass>,
    pub resource_bounds: ResourceBoundsMapping,
    pub tip: u64,
    pub paymaster_data: Vec<Felt>,
    pub account_deployment_data: Vec<Felt>,
    pub nonce_data_availability_mode: DataAvailabilityMode,
    pub fee_data_availability_mode: DataAvailabilityMode,
    pub is_query: bool,
}

impl BroadcastedDeclareTx {
    pub fn is_query(&self) -> bool {
        self.is_query
    }

    pub fn into_inner(
        self,
        chain_id: ChainId,
    ) -> Result<DeclareTxWithClass, BroadcastedDeclareTxError> {
        let class_hash = self.contract_class.hash()?;

        let rpc_class = Arc::unwrap_or_clone(self.contract_class);
        let class = ContractClass::Class(SierraContractClass::try_from(rpc_class)?);

        let tx = DeclareTx::V3(DeclareTxV3 {
            chain_id,
            class_hash,
            tip: self.tip,
            nonce: self.nonce,
            signature: self.signature,
            paymaster_data: self.paymaster_data,
            sender_address: self.sender_address,
            resource_bounds: self.resource_bounds,
            compiled_class_hash: self.compiled_class_hash,
            account_deployment_data: self.account_deployment_data,
            fee_data_availability_mode: self.fee_data_availability_mode,
            nonce_data_availability_mode: self.nonce_data_availability_mode,
        });

        Ok(DeclareTxWithClass::new(tx, class))
    }
}

impl serde::Serialize for BroadcastedDeclareTx {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;

        let version = if self.is_query { Felt::THREE + QUERY_VERSION_OFFSET } else { Felt::THREE };

        let mut state = serializer.serialize_struct("BroadcastedDeclareTx", 12)?;
        state.serialize_field("type", "DECLARE")?;
        state.serialize_field("version", &version)?;
        state.serialize_field("sender_address", &self.sender_address)?;
        state.serialize_field("compiled_class_hash", &self.compiled_class_hash)?;
        state.serialize_field("signature", &self.signature)?;
        state.serialize_field("nonce", &self.nonce)?;
        state.serialize_field("contract_class", &self.contract_class)?;
        state.serialize_field("resource_bounds", &self.resource_bounds)?;
        state.serialize_field("tip", &self.tip)?;
        state.serialize_field("paymaster_data", &self.paymaster_data)?;
        state.serialize_field("account_deployment_data", &self.account_deployment_data)?;
        state
            .serialize_field("nonce_data_availability_mode", &self.nonce_data_availability_mode)?;
        state.serialize_field("fee_data_availability_mode", &self.fee_data_availability_mode)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for BroadcastedDeclareTx {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        UntypedBroadcastedTx::deserialize(deserializer)?
            .try_into_declare()
            .map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone)]
pub struct BroadcastedDeployAccountTx {
    pub signature: Vec<Felt>,
    pub nonce: Nonce,
    pub contract_address_salt: Felt,
    pub constructor_calldata: Vec<Felt>,
    pub class_hash: ClassHash,
    pub resource_bounds: ResourceBoundsMapping,
    pub tip: u64,
    pub paymaster_data: Vec<Felt>,
    pub nonce_data_availability_mode: DataAvailabilityMode,
    pub fee_data_availability_mode: DataAvailabilityMode,
    pub is_query: bool,
}

impl BroadcastedDeployAccountTx {
    pub fn is_query(&self) -> bool {
        self.is_query
    }

    pub fn into_inner(self, chain_id: ChainId) -> DeployAccountTx {
        let contract_address = get_contract_address(
            self.contract_address_salt,
            self.class_hash,
            &self.constructor_calldata,
            Felt::ZERO,
        );

        DeployAccountTx::V3(DeployAccountTxV3 {
            chain_id,
            tip: self.tip,
            nonce: self.nonce,
            signature: self.signature,
            class_hash: self.class_hash,
            paymaster_data: self.paymaster_data,
            resource_bounds: self.resource_bounds,
            contract_address: contract_address.into(),
            constructor_calldata: self.constructor_calldata,
            contract_address_salt: self.contract_address_salt,
            fee_data_availability_mode: self.fee_data_availability_mode,
            nonce_data_availability_mode: self.nonce_data_availability_mode,
        })
    }
}

impl serde::Serialize for BroadcastedDeployAccountTx {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;

        let version = if self.is_query { Felt::THREE + QUERY_VERSION_OFFSET } else { Felt::THREE };

        let mut state = serializer.serialize_struct("BroadcastedDeployAccountTx", 11)?;
        state.serialize_field("type", "DEPLOY_ACCOUNT")?;
        state.serialize_field("version", &version)?;
        state.serialize_field("signature", &self.signature)?;
        state.serialize_field("nonce", &self.nonce)?;
        state.serialize_field("contract_address_salt", &self.contract_address_salt)?;
        state.serialize_field("constructor_calldata", &self.constructor_calldata)?;
        state.serialize_field("class_hash", &self.class_hash)?;
        state.serialize_field("resource_bounds", &self.resource_bounds)?;
        state.serialize_field("tip", &self.tip)?;
        state.serialize_field("paymaster_data", &self.paymaster_data)?;
        state
            .serialize_field("nonce_data_availability_mode", &self.nonce_data_availability_mode)?;
        state.serialize_field("fee_data_availability_mode", &self.fee_data_availability_mode)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for BroadcastedDeployAccountTx {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        UntypedBroadcastedTx::deserialize(deserializer)?
            .try_into_deploy_account()
            .map_err(de::Error::custom)
    }
}

/// Response for broadcasting an `INVOKE` transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddInvokeTransactionResult {
    /// The hash of the invoke transaction
    pub transaction_hash: TxHash,
}

/// Response for broadcasting a `DECLARE` transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddDeclareTransactionResult {
    /// The hash of the declare transaction
    pub transaction_hash: TxHash,
    /// The hash of the declared class
    pub class_hash: ClassHash,
}

/// Response for broadcasting a `DEPLOY_ACCOUNT` transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddDeployAccountTransactionResult {
    /// The hash of the deploy transaction
    pub transaction_hash: TxHash,
    /// The address of the new contract
    pub contract_address: ContractAddress,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum RpcResourceBoundsMapping {
    Current {
        #[serde(
            serialize_with = "serialize_resource_bounds",
            deserialize_with = "deserialize_resource_bounds"
        )]
        l1_gas: ResourceBounds,

        #[serde(
            serialize_with = "serialize_resource_bounds",
            deserialize_with = "deserialize_resource_bounds"
        )]
        l2_gas: ResourceBounds,

        #[serde(
            serialize_with = "serialize_resource_bounds",
            deserialize_with = "deserialize_resource_bounds"
        )]
        l1_data_gas: ResourceBounds,
    },

    Legacy {
        #[serde(
            serialize_with = "serialize_resource_bounds",
            deserialize_with = "deserialize_resource_bounds"
        )]
        l1_gas: ResourceBounds,

        #[serde(
            serialize_with = "serialize_resource_bounds",
            deserialize_with = "deserialize_resource_bounds"
        )]
        l2_gas: ResourceBounds,
    },
}

#[derive(Serialize, Deserialize)]
struct RpcResourceBounds {
    #[serde(serialize_with = "serialize_hex_u64", deserialize_with = "deserialize_hex_u64")]
    max_amount: u64,
    #[serde(serialize_with = "serialize_hex_u128", deserialize_with = "deserialize_hex_u128")]
    max_price_per_unit: u128,
}

fn serialize_resource_bounds_mapping<S: Serializer>(
    resource_bounds: &ResourceBoundsMapping,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let rpc_mapping = match resource_bounds {
        ResourceBoundsMapping::L1Gas(l1_gas) => RpcResourceBoundsMapping::Legacy {
            l1_gas: l1_gas.clone(),
            l2_gas: ResourceBounds::default(),
        },

        ResourceBoundsMapping::All(AllResourceBoundsMapping { l1_gas, l2_gas, l1_data_gas }) => {
            RpcResourceBoundsMapping::Current {
                l1_gas: l1_gas.clone(),
                l2_gas: l2_gas.clone(),
                l1_data_gas: l1_data_gas.clone(),
            }
        }
    };

    rpc_mapping.serialize(serializer)
}

fn deserialize_resource_bounds_mapping<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<ResourceBoundsMapping, D::Error> {
    let rpc_mapping = RpcResourceBoundsMapping::deserialize(deserializer)?;

    match rpc_mapping {
        RpcResourceBoundsMapping::Legacy { l1_gas, .. } => Ok(ResourceBoundsMapping::L1Gas(l1_gas)),
        RpcResourceBoundsMapping::Current { l1_gas, l2_gas, l1_data_gas } => {
            Ok(ResourceBoundsMapping::All(AllResourceBoundsMapping { l1_gas, l2_gas, l1_data_gas }))
        }
    }
}

fn serialize_resource_bounds<S: Serializer>(
    resource_bounds: &ResourceBounds,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let helper = RpcResourceBounds {
        max_amount: resource_bounds.max_amount,
        max_price_per_unit: resource_bounds.max_price_per_unit,
    };

    helper.serialize(serializer)
}

fn deserialize_resource_bounds<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<ResourceBounds, D::Error> {
    let helper = RpcResourceBounds::deserialize(deserializer)?;
    Ok(ResourceBounds {
        max_amount: helper.max_amount,
        max_price_per_unit: helper.max_price_per_unit,
    })
}

#[cfg(test)]
mod tests {
    use assert_matches::assert_matches;
    use katana_primitives::fee::ResourceBoundsMapping;
    use katana_primitives::transaction::TxType;
    use katana_primitives::{address, felt, ContractAddress, Felt};
    use serde_json::json;

    use super::*;
    use crate::broadcasted::{
        AddDeclareTransactionResult, AddDeployAccountTransactionResult, AddInvokeTransactionResult,
        BroadcastedTx, RpcResourceBoundsMapping, UntypedBroadcastedTxError,
    };

    #[test]
    fn legacy_rpc_resource_bounds_serde() {
        let json = json!({
            "l1_gas": {
                "max_amount": "0x1",
                "max_price_per_unit": "0x1"
            },
            "l2_gas": {
                "max_amount": "0x0",
                "max_price_per_unit": "0x0"
            }
        });

        let resource_bounds: RpcResourceBoundsMapping = serde_json::from_value(json).unwrap();

        assert_matches!(resource_bounds, RpcResourceBoundsMapping::Legacy { l1_gas, l2_gas } => {
            assert_eq!(l1_gas.max_amount, 1);
            assert_eq!(l1_gas.max_price_per_unit, 1);
            assert_eq!(l2_gas.max_amount, 0);
            assert_eq!(l2_gas.max_price_per_unit, 0);
        });
    }

    #[test]
    fn rpc_resource_bounds_serde() {
        let json = json!({
            "l2_gas": {
                "max_amount": "0xa",
                "max_price_per_unit": "0xb"
            },
            "l1_gas": {
                "max_amount": "0x1",
                "max_price_per_unit": "0x2"
            },
            "l1_data_gas": {
                "max_amount": "0xabc",
                "max_price_per_unit": "0x1337"
            }
        });

        let resource_bounds: RpcResourceBoundsMapping = serde_json::from_value(json).unwrap();

        assert_matches!(resource_bounds, RpcResourceBoundsMapping::Current {  l2_gas, l1_gas, l1_data_gas } => {
            assert_eq!(l2_gas.max_amount, 0xa);
            assert_eq!(l2_gas.max_price_per_unit, 0xb);

            assert_eq!(l1_gas.max_amount, 0x1);
            assert_eq!(l1_gas.max_price_per_unit, 0x2);

            assert_eq!(l1_data_gas.max_amount, 0xabc);
            assert_eq!(l1_data_gas.max_price_per_unit, 0x1337);
        });
    }

    #[test]
    fn untyped_invoke_tx_deserialization() {
        let json = json!({
          "account_deployment_data": [],
          "calldata": [
            "0x1",
            "0x3284bb4684703a368db8fd538c39b30e51822dbab9ad398e66311e820318444",
            "0xb6ed56333389f45e60acf4deb7dca7c0ab16a4b24e2e8797959bbea4063636",
            "0x2",
            "0x4",
            "0xcf"
          ],
          "fee_data_availability_mode": "L1",
          "nonce": "0x4c7",
          "nonce_data_availability_mode": "L1",
          "paymaster_data": [],
          "resource_bounds": {
            "l1_data_gas": {
              "max_amount": "0x360",
              "max_price_per_unit": "0x1626"
            },
            "l1_gas": {
              "max_amount": "0x0",
              "max_price_per_unit": "0x2fd0cf343294"
            },
            "l2_gas": {
              "max_amount": "0xc1e300",
              "max_price_per_unit": "0x4e575794"
            }
          },
          "sender_address": "0x7c93d1c768414c8acac9ef01e09d196ab097c44acec98bdbb9b2e4da37e8aaf",
          "signature": [
            "0x54f3f4b563f83ba5b0fdf339d07d0e88662e602db1243b5c23b4664d2530958",
            "0x4fb187a83219ade17afa01161359b0bb9f64da4be0f96ccefaaa6e41bae7dce"
          ],
          "tip": "0x0",
          "type": "INVOKE",
          "version": "0x3"
        });

        let untyped: UntypedBroadcastedTx = serde_json::from_value(json).unwrap();

        assert_eq!(untyped.version, Felt::THREE);
        assert_eq!(untyped.r#type, TxType::Invoke);

        // Common fields
        assert_eq!(untyped.tip, 0x0);
        assert_eq!(untyped.nonce, felt!("0x4c7"));
        assert_eq!(untyped.paymaster_data, vec![]);
        assert_eq!(untyped.fee_data_availability_mode, DataAvailabilityMode::L1);
        assert_eq!(untyped.nonce_data_availability_mode, DataAvailabilityMode::L1);
        assert_matches!(&untyped.resource_bounds, ResourceBoundsMapping::All(bounds) => {
            assert_eq!(bounds.l1_gas.max_amount, 0x0);
            assert_eq!(bounds.l1_gas.max_price_per_unit, 0x2fd0cf343294);

            assert_eq!(bounds.l2_gas.max_amount, 0xc1e300);
            assert_eq!(bounds.l2_gas.max_price_per_unit, 0x4e575794);

            assert_eq!(bounds.l1_data_gas.max_amount, 0x360);
            assert_eq!(bounds.l1_data_gas.max_price_per_unit, 0x1626);
        });
        assert_eq!(
            untyped.signature,
            vec![
                felt!("0x54f3f4b563f83ba5b0fdf339d07d0e88662e602db1243b5c23b4664d2530958"),
                felt!("0x4fb187a83219ade17afa01161359b0bb9f64da4be0f96ccefaaa6e41bae7dce")
            ]
        );

        // Tx specific fields
        assert_eq!(
            untyped.sender_address,
            Some(address!("0x7c93d1c768414c8acac9ef01e09d196ab097c44acec98bdbb9b2e4da37e8aaf"))
        );
        assert_eq!(
            untyped.calldata,
            Some(vec![
                felt!("0x1"),
                felt!("0x3284bb4684703a368db8fd538c39b30e51822dbab9ad398e66311e820318444"),
                felt!("0xb6ed56333389f45e60acf4deb7dca7c0ab16a4b24e2e8797959bbea4063636"),
                felt!("0x2"),
                felt!("0x4"),
                felt!("0xcf")
            ])
        );

        // Declare Tx fields should be None
        assert!(untyped.contract_class.is_none());
        assert!(untyped.compiled_class_hash.is_none());

        // DeployAccount Tx fields should be None
        assert!(untyped.class_hash.is_none());
        assert!(untyped.constructor_calldata.is_none());
        assert!(untyped.contract_address_salt.is_none());

        // Make sure can convert to the correct typed tx.
        let typed = untyped.clone().typed().expect("failed to convert to typed tx");
        assert_matches!(typed, BroadcastedTx::Invoke(tx) => {
            // common fields
            assert_eq!(tx.tip, untyped.tip);
            assert_eq!(tx.nonce, untyped.nonce);
            assert_eq!(tx.signature, untyped.signature);
            assert_eq!(tx.paymaster_data, untyped.paymaster_data);
            assert_eq!(tx.fee_data_availability_mode, untyped.fee_data_availability_mode);
            assert_eq!(tx.nonce_data_availability_mode, untyped.nonce_data_availability_mode);

            // tx specific fields
            assert_eq!(tx.calldata,  untyped.calldata.unwrap());
            assert_eq!(tx.sender_address, untyped.sender_address.unwrap());
        });
    }

    #[test]
    fn untyped_declare_tx_deserialization() {
        let json = json!({
            "type": "DECLARE",
            "version": "0x3",
            "signature": ["0x1"],
            "tip": "0x0",
            "paymaster_data": [],
            "resource_bounds": {
                "l1_gas": {
                    "max_amount": "0x100",
                    "max_price_per_unit": "0x200"
                },
                "l2_gas": {
                    "max_amount": "0x0",
                    "max_price_per_unit": "0x0"
                }
            },
            "nonce_data_availability_mode": "L1",
            "fee_data_availability_mode": "L1",
            "sender_address": "0x456",
            "compiled_class_hash": "0x789",
            "nonce": "0x1",
            "account_deployment_data": ["0x11", "0x22"],
            "contract_class": {
                "sierra_program": ["0x1", "0x2"],
                "contract_class_version": "0.1.0",
                "entry_points_by_type": {
                    "EXTERNAL": [],
                    "L1_HANDLER": [],
                    "CONSTRUCTOR": []
                },
                "abi": ""
            }
        });

        let untyped: UntypedBroadcastedTx = serde_json::from_value(json).unwrap();

        assert_eq!(untyped.version, Felt::THREE);
        assert_eq!(untyped.r#type, TxType::Declare);

        // Common fields
        assert_eq!(untyped.tip, 0x0);
        assert_eq!(untyped.nonce, felt!("0x1"));
        assert_eq!(untyped.paymaster_data, vec![]);
        assert_eq!(untyped.fee_data_availability_mode, DataAvailabilityMode::L1);
        assert_eq!(untyped.nonce_data_availability_mode, DataAvailabilityMode::L1);
        assert_matches!(&untyped.resource_bounds, ResourceBoundsMapping::L1Gas(bounds) => {
            assert_eq!(bounds.max_amount, 0x100);
            assert_eq!(bounds.max_price_per_unit, 0x200);
        });

        // Tx specific fields
        assert!(untyped.contract_class.is_some());
        assert_eq!(untyped.sender_address, Some(address!("0x456")));
        assert_eq!(untyped.compiled_class_hash, Some(felt!("0x789")));
        assert_eq!(untyped.account_deployment_data, Some(vec![felt!("0x11"), felt!("0x22")]));

        // Invoke Tx fields should be None
        assert!(untyped.calldata.is_none());

        // DeployAccount Tx fields should be None
        assert!(untyped.class_hash.is_none());
        assert!(untyped.constructor_calldata.is_none());
        assert!(untyped.contract_address_salt.is_none());

        // Make sure can convert to the correct typed tx.
        let typed = untyped.clone().typed().expect("failed to convert to typed tx");
        assert_matches!(typed, BroadcastedTx::Declare(tx) => {
            // common fields
            assert_eq!(tx.tip, untyped.tip);
            assert_eq!(tx.nonce, untyped.nonce);
            assert_eq!(tx.signature, untyped.signature);
            assert_eq!(tx.paymaster_data, untyped.paymaster_data);
            assert_eq!(tx.fee_data_availability_mode, untyped.fee_data_availability_mode);
            assert_eq!(tx.nonce_data_availability_mode, untyped.nonce_data_availability_mode);

            // tx specific fields
            assert_eq!(tx.sender_address, untyped.sender_address.unwrap());
            assert_eq!(tx.compiled_class_hash, untyped.compiled_class_hash.unwrap());
        });
    }

    #[test]
    fn untyped_deploy_account_tx_deserialization() {
        let json = json!({
            "type": "DEPLOY_ACCOUNT",
            "version": "0x3",
            "signature": ["0x1", "0x2", "0x3"],
            "tip": "0x1",
            "paymaster_data": [],
            "resource_bounds": {
                "l1_gas": {
                    "max_amount": "0x100",
                    "max_price_per_unit": "0x200"
                },
                "l2_gas": {
                    "max_amount": "0x300",
                    "max_price_per_unit": "0x400"
                }
            },
            "nonce_data_availability_mode": "L1",
            "fee_data_availability_mode": "L1",
            "nonce": "0x0",
            "contract_address_salt": "0xabc",
            "constructor_calldata": ["0x1", "0x2"],
            "class_hash": "0xdef"
        });

        let untyped: UntypedBroadcastedTx = serde_json::from_value(json).unwrap();

        assert_eq!(untyped.version, Felt::THREE);
        assert_eq!(untyped.r#type, TxType::DeployAccount);

        // Common fields
        assert_eq!(untyped.tip, 0x1);
        assert_eq!(untyped.nonce, felt!("0x0"));
        assert_eq!(untyped.paymaster_data, vec![]);
        assert_eq!(untyped.signature, vec![felt!("0x1"), felt!("0x2"), felt!("0x3")]);
        assert_eq!(untyped.fee_data_availability_mode, DataAvailabilityMode::L1);
        assert_eq!(untyped.nonce_data_availability_mode, DataAvailabilityMode::L1);
        assert_matches!(&untyped.resource_bounds, ResourceBoundsMapping::L1Gas(bounds) => {
            assert_eq!(bounds.max_amount, 0x100);
            assert_eq!(bounds.max_price_per_unit, 0x200);
        });

        // Tx specific fields
        assert_eq!(untyped.class_hash, Some(felt!("0xdef")));
        assert_eq!(untyped.contract_address_salt, Some(felt!("0xabc")));
        assert_eq!(untyped.constructor_calldata, Some(vec![felt!("0x1"), felt!("0x2")]));

        // Invoke Tx fields should be None
        assert!(untyped.sender_address.is_none());
        assert!(untyped.calldata.is_none());
        assert!(untyped.account_deployment_data.is_none());

        // Declare Tx fields should be None
        assert!(untyped.compiled_class_hash.is_none());
        assert!(untyped.contract_class.is_none());

        // Make sure can convert to the correct typed tx.
        let typed = untyped.clone().typed().expect("failed to convert to typed tx");
        assert_matches!(typed, BroadcastedTx::DeployAccount(tx) => {
            // common fields
            assert_eq!(tx.tip, untyped.tip);
            assert_eq!(tx.nonce, untyped.nonce);
            assert_eq!(tx.signature, untyped.signature);
            assert_eq!(tx.paymaster_data, untyped.paymaster_data);
            assert_eq!(tx.fee_data_availability_mode, untyped.fee_data_availability_mode);
            assert_eq!(tx.nonce_data_availability_mode, untyped.nonce_data_availability_mode);

            // tx specific fields
            assert_eq!(tx.class_hash, untyped.class_hash.unwrap());
            assert_eq!(tx.constructor_calldata, untyped.constructor_calldata.unwrap());
            assert_eq!(tx.contract_address_salt,  untyped.contract_address_salt.unwrap());
        });
    }

    #[test]
    fn query_version_handling() {
        let query_version = Felt::THREE + QUERY_VERSION_OFFSET;
        let json = json!({
            "type": "INVOKE",
            "version": query_version,
            "signature": [],
            "tip": "0x0",
            "paymaster_data": [],
            "resource_bounds": {
                "l1_gas": {
                    "max_amount": "0x100",
                    "max_price_per_unit": "0x200"
                },
                "l2_gas": {
                    "max_amount": "0x0",
                    "max_price_per_unit": "0x0"
                }
            },
            "nonce_data_availability_mode": "L1",
            "fee_data_availability_mode": "L1",
            "sender_address": "0x123",
            "calldata": [],
            "nonce": "0x0",
            "account_deployment_data": []
        });

        let tx: UntypedBroadcastedTx = serde_json::from_value(json).unwrap();
        let invoke_tx = tx.try_into_invoke().unwrap();
        assert!(invoke_tx.is_query());
    }

    #[test]
    fn broadcasted_tx_enum_deserialization() {
        let json = json!({
            "type": "INVOKE",
            "version": "0x3",
            "signature": [],
            "tip": "0x0",
            "paymaster_data": [],
            "resource_bounds": {
                "l1_gas": {
                    "max_amount": "0x100",
                    "max_price_per_unit": "0x200"
                },
                "l2_gas": {
                    "max_amount": "0x0",
                    "max_price_per_unit": "0x0"
                }
            },
            "nonce_data_availability_mode": "L1",
            "fee_data_availability_mode": "L1",
            "sender_address": "0x123",
            "calldata": [],
            "nonce": "0x0",
            "account_deployment_data": []
        });

        let tx: BroadcastedTx = serde_json::from_value(json).unwrap();
        assert_matches!(tx, BroadcastedTx::Invoke(_));
    }

    // Attempting to deserialize a broadcasted transaction that is missing a
    // field that is not specific to a type.
    #[test]
    fn missing_common_field_error() {
        // Invoke transaction but missing a common field: `signature`
        let json = json!({
            "type": "INVOKE",
            "version": "0x3",
            "tip": "0xa",
            "paymaster_data": [],
            "sender_address": "0x123",
            "calldata": ["0x1", "0x2", "0x3"],
            "nonce": "0x5",
            "account_deployment_data": [],
            "fee_data_availability_mode": "L1",
            "nonce_data_availability_mode": "L1",
            "resource_bounds": {
                "l1_gas": {
                    "max_amount": "0x100",
                    "max_price_per_unit": "0x200"
                },
                "l2_gas": {
                    "max_amount": "0x300",
                    "max_price_per_unit": "0x400"
                }
            },
        });

        assert!(serde_json::from_value::<UntypedBroadcastedTx>(json).is_err())
    }

    // Attempting to convert to a typed transaction but missing a field that
    // is specific to that type.
    #[test]
    fn missing_field_error() {
        // Invoke transaction but missing a tx specific field: `calldata`
        let json = json!({
            "type": "INVOKE",
            "version": "0x3",
            "nonce": "0x3",
            "signature": [],
            "tip": "0x0",
            "paymaster_data": [],
            "sender_address": "0x1337",
            "resource_bounds": {
                "l1_gas": {
                    "max_amount": "0x100",
                    "max_price_per_unit": "0x200"
                },
                "l2_gas": {
                    "max_amount": "0x0",
                    "max_price_per_unit": "0x0"
                }
            },
            "nonce_data_availability_mode": "L1",
            "fee_data_availability_mode": "L1",
            "account_deployment_data": ["0x11", "0x22"]
            // "calldata": ["0x1", "0x67"],
        });

        let tx: UntypedBroadcastedTx = serde_json::from_value(json).unwrap();
        let result = tx.try_into_invoke();

        assert_matches!(
            result,
            Err(UntypedBroadcastedTxError::MissingField {
                r#type: TxType::Invoke,
                field: "calldata"
            })
        );
    }

    #[test]
    fn invalid_version_error() {
        let json = json!({
            "type": "INVOKE",
            "version": "0x2", // Invalid version
            "signature": [],
            "tip": "0x0",
            "paymaster_data": [],
            "resource_bounds": {
                "l1_gas": {
                    "max_amount": "0x100",
                    "max_price_per_unit": "0x200"
                },
                "l2_gas": {
                    "max_amount": "0x0",
                    "max_price_per_unit": "0x0"
                }
            },
            "nonce_data_availability_mode": "L1",
            "fee_data_availability_mode": "L1",
            "sender_address": "0x123",
            "calldata": [],
            "nonce": "0x0",
            "account_deployment_data": []
        });

        let tx: UntypedBroadcastedTx = serde_json::from_value(json).unwrap();
        let result = tx.try_into_invoke();

        assert_matches!(
            result,
            Err(UntypedBroadcastedTxError::InvalidVersion { r#type: TxType::Invoke, version }) => {
                assert_eq!(version, Felt::TWO);
            }
        );
    }

    #[test]
    fn invalid_tx_type_error() {
        let json = json!({
          "account_deployment_data": [],
          "calldata": [
            "0x1",
            "0x3284bb4684703a368db8fd538c39b30e51822dbab9ad398e66311e820318444",
            "0xb6ed56333389f45e60acf4deb7dca7c0ab16a4b24e2e8797959bbea4063636",
            "0x2",
            "0x4",
            "0xcf"
          ],
          "fee_data_availability_mode": "L1",
          "nonce": "0x4c7",
          "nonce_data_availability_mode": "L1",
          "paymaster_data": [],
          "resource_bounds": {
            "l1_data_gas": {
              "max_amount": "0x360",
              "max_price_per_unit": "0x1626"
            },
            "l1_gas": {
              "max_amount": "0x0",
              "max_price_per_unit": "0x2fd0cf343294"
            },
            "l2_gas": {
              "max_amount": "0xc1e300",
              "max_price_per_unit": "0x4e575794"
            }
          },
          "sender_address": "0x7c93d1c768414c8acac9ef01e09d196ab097c44acec98bdbb9b2e4da37e8aaf",
          "signature": [
            "0x54f3f4b563f83ba5b0fdf339d07d0e88662e602db1243b5c23b4664d2530958",
            "0x4fb187a83219ade17afa01161359b0bb9f64da4be0f96ccefaaa6e41bae7dce"
          ],
          "tip": "0x0",
          "type": "INVOKE",
          "version": "0x3"
        });

        let tx: UntypedBroadcastedTx = serde_json::from_value(json).unwrap();
        let invoke_tx_result = tx.clone().try_into_invoke();
        let declare_tx_result = tx.clone().try_into_declare();
        let deploy_account_tx_result = tx.try_into_deploy_account();

        assert!(invoke_tx_result.is_ok());

        assert_matches!(
            declare_tx_result,
            Err(UntypedBroadcastedTxError::InvalidTxType {
                expected: TxType::Declare,
                actual: TxType::Invoke
            })
        );
        assert_matches!(
            deploy_account_tx_result,
            Err(UntypedBroadcastedTxError::InvalidTxType {
                expected: TxType::DeployAccount,
                actual: TxType::Invoke
            })
        );
    }

    #[test]
    fn response_types_serialization() {
        let invoke_result = AddInvokeTransactionResult { transaction_hash: felt!("0x123") };

        let expected = json!({
            "transaction_hash": "0x123"
        });

        let actual = serde_json::to_value(&invoke_result).unwrap();
        similar_asserts::assert_eq!(actual, expected);

        let declare_result = AddDeclareTransactionResult {
            transaction_hash: felt!("0x456"),
            class_hash: felt!("0x789"),
        };

        let expected = json!({
            "transaction_hash": "0x456",
            "class_hash": "0x789"
        });

        let actual = serde_json::to_value(&declare_result).unwrap();
        similar_asserts::assert_eq!(actual, expected);

        let deploy_result = AddDeployAccountTransactionResult {
            transaction_hash: felt!("0xabc"),
            contract_address: address!("0xdef"),
        };

        let expected = json!({
            "transaction_hash": "0xabc",
            "contract_address": "0xdef"
        });

        let actual = serde_json::to_value(&deploy_result).unwrap();
        similar_asserts::assert_eq!(actual, expected);
    }
}
