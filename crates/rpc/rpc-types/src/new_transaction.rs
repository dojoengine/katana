use std::sync::Arc;

use katana_primitives::chain::ChainId;
use katana_primitives::class::{ClassHash, CompiledClassHash, ContractClass, SierraContractClass};
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

    #[error("invalid version for {r#type} transaction")]
    InvalidVersion { r#type: TxType },
}

#[serde_with::serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UntypedBroadcastedTx {
    pub r#type: TxType,
    pub version: Felt,
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

    // Invoke-specific fields
    #[serde(default)]
    pub sender_address: Option<ContractAddress>,
    #[serde(default)]
    pub calldata: Option<Vec<Felt>>,
    #[serde(default)]
    pub nonce: Option<Felt>,
    #[serde(default)]
    pub account_deployment_data: Option<Vec<Felt>>,

    // Declare-specific fields
    #[serde(default)]
    pub compiled_class_hash: Option<CompiledClassHash>,
    #[serde(default)]
    pub contract_class: Option<Arc<RpcSierraContractClass>>,

    // DeployAccount-specific fields
    #[serde(default)]
    pub contract_address_salt: Option<Felt>,
    #[serde(default)]
    pub constructor_calldata: Option<Vec<Felt>>,
    #[serde(default)]
    pub class_hash: Option<ClassHash>,
}

impl UntypedBroadcastedTx {
    pub fn typed(self) -> BroadcastedTx {
        unimplemented!()
    }

    pub fn try_into_invoke_tx(self) -> Result<BroadcastedInvokeTx, UntypedBroadcastedTxError> {
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
            return Err(UntypedBroadcastedTxError::InvalidVersion { r#type: TxType::Invoke });
        };

        let nonce = expect_field!(self.nonce, TxType::Invoke, "nonce");
        let calldata = expect_field!(self.calldata, TxType::Invoke, "calldata");
        let sender_address = expect_field!(self.sender_address, TxType::Invoke, "sender_address");
        let account_deployment_data =
            expect_field!(self.account_deployment_data, TxType::Invoke, "account_deployment_data");

        Ok(BroadcastedInvokeTx {
            nonce,
            calldata,
            tip: self.tip,
            sender_address,
            account_deployment_data,
            signature: self.signature,
            paymaster_data: self.paymaster_data,
            resource_bounds: self.resource_bounds,
            fee_data_availability_mode: self.fee_data_availability_mode,
            nonce_data_availability_mode: self.nonce_data_availability_mode,
            is_query,
        })
    }

    pub fn try_into_declare_tx(self) -> Result<BroadcastedDeclareTx, UntypedBroadcastedTxError> {
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
            return Err(UntypedBroadcastedTxError::InvalidVersion { r#type: TxType::Declare });
        };

        let sender_address = expect_field!(self.sender_address, TxType::Declare, "sender_address");
        let compiled_class_hash =
            expect_field!(self.compiled_class_hash, TxType::Declare, "compiled_class_hash");
        let nonce = expect_field!(self.nonce, TxType::Declare, "nonce");
        let contract_class = expect_field!(self.contract_class, TxType::Declare, "contract_class");
        let account_deployment_data =
            expect_field!(self.account_deployment_data, TxType::Declare, "account_deployment_data");

        Ok(BroadcastedDeclareTx {
            nonce,
            contract_class,
            sender_address,
            tip: self.tip,
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

    pub fn try_into_deploy_account_tx(
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
                r#type: TxType::DeployAccount,
            });
        };

        let nonce = expect_field!(self.nonce, TxType::DeployAccount, "nonce");
        let contract_address_salt = expect_field!(
            self.contract_address_salt,
            TxType::DeployAccount,
            "contract_address_salt"
        );
        let constructor_calldata =
            expect_field!(self.constructor_calldata, TxType::DeployAccount, "constructor_calldata");
        let class_hash = expect_field!(self.class_hash, TxType::DeployAccount, "class_hash");

        Ok(BroadcastedDeployAccountTx {
            nonce,
            class_hash,
            tip: self.tip,
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
        let untyped = UntypedBroadcastedTx::deserialize(deserializer)?;

        match untyped.r#type {
            TxType::Invoke => {
                let typed = untyped.try_into_invoke_tx().map_err(de::Error::custom)?;
                Ok(BroadcastedTx::Invoke(typed))
            }

            TxType::Declare => {
                let typed = untyped.try_into_declare_tx().map_err(de::Error::custom)?;
                Ok(BroadcastedTx::Declare(typed))
            }

            TxType::DeployAccount => {
                let typed = untyped.try_into_deploy_account_tx().map_err(de::Error::custom)?;
                Ok(BroadcastedTx::DeployAccount(typed))
            }

            r#type @ _ => Err(de::Error::custom(format!("unsupported transaction type {type}"))),
        }
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
        let untyped = UntypedBroadcastedTx::deserialize(deserializer)?;

        let is_query = if dbg!(untyped.version) == Felt::THREE {
            false
        } else if untyped.version == Felt::THREE + QUERY_VERSION_OFFSET {
            true
        } else {
            return Err(serde::de::Error::custom("invalid `version` value"));
        };

        Ok(dbg!(Self { is_query, ..untyped.try_into_invoke_tx().unwrap() }))
    }
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

    pub fn into_inner(self, chain_id: ChainId) -> DeclareTxWithClass {
        let class_hash = self.contract_class.hash().unwrap();

        let rpc_class = Arc::unwrap_or_clone(self.contract_class);
        let class = ContractClass::Class(SierraContractClass::try_from(rpc_class).unwrap());

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

        DeclareTxWithClass::new(tx, class)
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
        let untyped = UntypedBroadcastedTx::deserialize(deserializer)?;

        let is_query = if untyped.version == Felt::THREE {
            false
        } else if untyped.version == Felt::THREE + QUERY_VERSION_OFFSET {
            true
        } else {
            return Err(serde::de::Error::custom("invalid `version` value"));
        };

        Ok(Self { is_query, ..untyped.try_into_declare_tx().unwrap() })
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
        let untyped = UntypedBroadcastedTx::deserialize(deserializer)?;

        let is_query = if untyped.version == Felt::THREE {
            false
        } else if untyped.version == Felt::THREE + QUERY_VERSION_OFFSET {
            true
        } else {
            return Err(serde::de::Error::custom("invalid `version` value"));
        };

        Ok(Self { is_query, ..untyped.try_into_deploy_account_tx().unwrap() })
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
    use serde_json::json;

    use crate::new_transaction::RpcResourceBoundsMapping;

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
}
