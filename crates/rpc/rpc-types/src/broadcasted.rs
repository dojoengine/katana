use std::sync::Arc;

use katana_pool_api::PoolTransaction;
use katana_primitives::chain::ChainId;
use katana_primitives::class::{
    ClassHash, CompiledClassHash, ComputeClassHashError, ContractClass, SierraContractClass,
};
use katana_primitives::contract::Nonce;
use katana_primitives::da::DataAvailabilityMode;
use katana_primitives::fee::{
    L1GasResourceBoundsMapping, ResourceBounds, ResourceBoundsMapping, Tip,
};
use katana_primitives::transaction::{
    DeclareTx, DeclareTxV0, DeclareTxV1, DeclareTxV2, DeclareTxV3, DeclareTxWithClass,
    DeployAccountTx, DeployAccountTxV1, DeployAccountTxV3, ExecutableTx, ExecutableTxWithHash,
    InvokeTx, InvokeTxV0, InvokeTxV1, InvokeTxV3, TxHash, TxType,
};
use katana_primitives::utils::get_contract_address;
use katana_primitives::{ContractAddress, Felt};
use serde::{de, Deserialize, Deserializer, Serialize};

use crate::class::SierraClass;

pub const QUERY_VERSION_OFFSET: Felt =
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

/// An untyped broadcasted transaction.
///
/// This type can be used to represent any type of transaction, and can be converted to a typed
/// transaction provided that all the required fields for that particular transaction type are
/// present.
#[serde_with::serde_as]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UntypedBroadcastedTx {
    pub r#type: TxType,
    pub version: Felt,
    pub nonce: Nonce,
    pub signature: Vec<Felt>,
    pub tip: Tip,
    pub paymaster_data: Vec<Felt>,
    pub resource_bounds: ResourceBoundsMapping,
    pub nonce_data_availability_mode: DataAvailabilityMode,
    pub fee_data_availability_mode: DataAvailabilityMode,

    // Invoke only fields
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calldata: Option<Vec<Felt>>,

    // Declare only fields
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compiled_class_hash: Option<CompiledClassHash>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract_class: Option<Arc<SierraClass>>,

    // Invoke & Declare only field
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_address: Option<ContractAddress>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_deployment_data: Option<Vec<Felt>>,

    // DeployAccount only fields
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract_address_salt: Option<Felt>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constructor_calldata: Option<Vec<Felt>>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
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

impl From<BroadcastedTx> for UntypedBroadcastedTx {
    fn from(tx: BroadcastedTx) -> Self {
        match tx {
            BroadcastedTx::Invoke(tx) => tx.into(),
            BroadcastedTx::Declare(tx) => tx.into(),
            BroadcastedTx::DeployAccount(tx) => tx.into(),
        }
    }
}

impl From<BroadcastedInvokeTx> for UntypedBroadcastedTx {
    fn from(tx: BroadcastedInvokeTx) -> Self {
        let version = if tx.is_query { Felt::THREE + QUERY_VERSION_OFFSET } else { Felt::THREE };

        Self {
            r#type: TxType::Invoke,
            version,
            tip: tx.tip,
            nonce: tx.nonce,
            signature: tx.signature,
            paymaster_data: tx.paymaster_data,
            resource_bounds: tx.resource_bounds,
            sender_address: Some(tx.sender_address),
            account_deployment_data: Some(tx.account_deployment_data),
            fee_data_availability_mode: tx.fee_data_availability_mode,
            nonce_data_availability_mode: tx.nonce_data_availability_mode,
            calldata: Some(tx.calldata),
            class_hash: None,
            contract_class: None,
            compiled_class_hash: None,
            constructor_calldata: None,
            contract_address_salt: None,
        }
    }
}

impl From<BroadcastedDeclareTx> for UntypedBroadcastedTx {
    fn from(tx: BroadcastedDeclareTx) -> Self {
        let version = if tx.is_query { Felt::THREE + QUERY_VERSION_OFFSET } else { Felt::THREE };

        Self {
            r#type: TxType::Declare,
            version,
            tip: tx.tip,
            nonce: tx.nonce,
            signature: tx.signature,
            paymaster_data: tx.paymaster_data,
            resource_bounds: tx.resource_bounds,
            contract_class: Some(tx.contract_class),
            sender_address: Some(tx.sender_address),
            compiled_class_hash: Some(tx.compiled_class_hash),
            account_deployment_data: Some(tx.account_deployment_data),
            fee_data_availability_mode: tx.fee_data_availability_mode,
            nonce_data_availability_mode: tx.nonce_data_availability_mode,
            calldata: None,
            class_hash: None,
            constructor_calldata: None,
            contract_address_salt: None,
        }
    }
}

impl From<BroadcastedDeployAccountTx> for UntypedBroadcastedTx {
    fn from(tx: BroadcastedDeployAccountTx) -> Self {
        let version = if tx.is_query { Felt::THREE + QUERY_VERSION_OFFSET } else { Felt::THREE };

        Self {
            r#type: TxType::DeployAccount,
            version,
            tip: tx.tip,
            nonce: tx.nonce,
            signature: tx.signature,
            class_hash: Some(tx.class_hash),
            paymaster_data: tx.paymaster_data,
            resource_bounds: tx.resource_bounds,
            constructor_calldata: Some(tx.constructor_calldata),
            contract_address_salt: Some(tx.contract_address_salt),
            fee_data_availability_mode: tx.fee_data_availability_mode,
            nonce_data_availability_mode: tx.nonce_data_availability_mode,
            calldata: None,
            contract_class: None,
            sender_address: None,
            compiled_class_hash: None,
            account_deployment_data: None,
        }
    }
}

// From implementations for primitive types to broadcasted types

impl From<InvokeTx> for BroadcastedInvokeTx {
    fn from(tx: InvokeTx) -> Self {
        match tx {
            InvokeTx::V0(tx) => tx.into(),
            InvokeTx::V1(tx) => tx.into(),
            InvokeTx::V3(tx) => tx.into(),
        }
    }
}

impl From<InvokeTxV0> for BroadcastedInvokeTx {
    fn from(tx: InvokeTxV0) -> Self {
        BroadcastedInvokeTx {
            sender_address: tx.contract_address,
            calldata: tx.calldata,
            signature: tx.signature,
            nonce: Felt::ZERO,
            paymaster_data: vec![],
            tip: 0.into(),
            account_deployment_data: vec![],
            resource_bounds: ResourceBoundsMapping::L1Gas(L1GasResourceBoundsMapping {
                l1_gas: ResourceBounds { max_amount: 0, max_price_per_unit: tx.max_fee },
                l2_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
            }),
            fee_data_availability_mode: DataAvailabilityMode::L1,
            nonce_data_availability_mode: DataAvailabilityMode::L1,
            is_query: false,
        }
    }
}

impl From<InvokeTxV1> for BroadcastedInvokeTx {
    fn from(tx: InvokeTxV1) -> Self {
        BroadcastedInvokeTx {
            sender_address: tx.sender_address,
            calldata: tx.calldata,
            signature: tx.signature,
            nonce: tx.nonce,
            paymaster_data: vec![],
            tip: 0.into(),
            account_deployment_data: vec![],
            resource_bounds: ResourceBoundsMapping::L1Gas(L1GasResourceBoundsMapping {
                l1_gas: ResourceBounds { max_amount: 0, max_price_per_unit: tx.max_fee },
                l2_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
            }),
            fee_data_availability_mode: DataAvailabilityMode::L1,
            nonce_data_availability_mode: DataAvailabilityMode::L1,
            is_query: false,
        }
    }
}

impl From<InvokeTxV3> for BroadcastedInvokeTx {
    fn from(tx: InvokeTxV3) -> Self {
        BroadcastedInvokeTx {
            sender_address: tx.sender_address,
            calldata: tx.calldata,
            signature: tx.signature,
            nonce: tx.nonce,
            paymaster_data: tx.paymaster_data,
            tip: tx.tip.into(),
            account_deployment_data: tx.account_deployment_data,
            resource_bounds: tx.resource_bounds,
            fee_data_availability_mode: tx.fee_data_availability_mode,
            nonce_data_availability_mode: tx.nonce_data_availability_mode,
            is_query: false,
        }
    }
}

impl From<DeclareTx> for BroadcastedDeclareTx {
    fn from(tx: DeclareTx) -> Self {
        match tx {
            DeclareTx::V0(tx) => tx.into(),
            DeclareTx::V1(tx) => tx.into(),
            DeclareTx::V2(tx) => tx.into(),
            DeclareTx::V3(tx) => tx.into(),
        }
    }
}

impl From<DeclareTxV0> for BroadcastedDeclareTx {
    fn from(tx: DeclareTxV0) -> Self {
        // Note: V0 declare transactions use legacy classes, but BroadcastedDeclareTx expects
        // Sierra. The actual contract class needs to be provided separately.
        BroadcastedDeclareTx {
            sender_address: tx.sender_address,
            compiled_class_hash: CompiledClassHash::ZERO,
            signature: tx.signature,
            nonce: Felt::ZERO,
            contract_class: Arc::new(SierraClass::default()),
            paymaster_data: vec![],
            tip: 0.into(),
            account_deployment_data: vec![],
            resource_bounds: ResourceBoundsMapping::L1Gas(L1GasResourceBoundsMapping {
                l1_gas: ResourceBounds { max_amount: 0, max_price_per_unit: tx.max_fee },
                l2_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
            }),
            fee_data_availability_mode: DataAvailabilityMode::L1,
            nonce_data_availability_mode: DataAvailabilityMode::L1,
            is_query: false,
        }
    }
}

impl From<DeclareTxV1> for BroadcastedDeclareTx {
    fn from(tx: DeclareTxV1) -> Self {
        // Note: V1 declare transactions use legacy classes, but BroadcastedDeclareTx expects
        // Sierra. The actual contract class needs to be provided separately.
        BroadcastedDeclareTx {
            sender_address: tx.sender_address,
            compiled_class_hash: CompiledClassHash::ZERO,
            signature: tx.signature,
            nonce: tx.nonce,
            contract_class: Arc::new(SierraClass::default()),
            paymaster_data: vec![],
            tip: 0.into(),
            account_deployment_data: vec![],
            resource_bounds: ResourceBoundsMapping::L1Gas(L1GasResourceBoundsMapping {
                l1_gas: ResourceBounds { max_amount: 0, max_price_per_unit: tx.max_fee },
                l2_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
            }),
            fee_data_availability_mode: DataAvailabilityMode::L1,
            nonce_data_availability_mode: DataAvailabilityMode::L1,
            is_query: false,
        }
    }
}

impl From<DeclareTxV2> for BroadcastedDeclareTx {
    fn from(tx: DeclareTxV2) -> Self {
        BroadcastedDeclareTx {
            sender_address: tx.sender_address,
            compiled_class_hash: tx.compiled_class_hash,
            signature: tx.signature,
            nonce: tx.nonce,
            contract_class: Arc::new(SierraClass::default()),
            paymaster_data: vec![],
            tip: 0.into(),
            account_deployment_data: vec![],
            resource_bounds: ResourceBoundsMapping::L1Gas(L1GasResourceBoundsMapping {
                l1_gas: ResourceBounds { max_amount: 0, max_price_per_unit: tx.max_fee },
                l2_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
            }),
            fee_data_availability_mode: DataAvailabilityMode::L1,
            nonce_data_availability_mode: DataAvailabilityMode::L1,
            is_query: false,
        }
    }
}

impl From<DeclareTxV3> for BroadcastedDeclareTx {
    fn from(tx: DeclareTxV3) -> Self {
        BroadcastedDeclareTx {
            sender_address: tx.sender_address,
            compiled_class_hash: tx.compiled_class_hash,
            signature: tx.signature,
            nonce: tx.nonce,
            contract_class: Arc::new(SierraClass::default()),
            paymaster_data: tx.paymaster_data,
            tip: tx.tip.into(),
            account_deployment_data: tx.account_deployment_data,
            resource_bounds: tx.resource_bounds,
            fee_data_availability_mode: tx.fee_data_availability_mode,
            nonce_data_availability_mode: tx.nonce_data_availability_mode,
            is_query: false,
        }
    }
}

impl From<DeclareTxWithClass> for BroadcastedDeclareTx {
    fn from(tx: DeclareTxWithClass) -> Self {
        // Convert ContractClass to SierraClass
        // Note: This conversion may lose information for legacy classes
        let contract_class = match tx.class.as_ref() {
            ContractClass::Class(sierra) => {
                Arc::new(SierraClass::try_from(sierra.clone()).unwrap_or_default())
            }
            ContractClass::Legacy(_) => Arc::new(SierraClass::default()),
        };

        match tx.transaction {
            DeclareTx::V0(tx) => BroadcastedDeclareTx {
                sender_address: tx.sender_address,
                compiled_class_hash: CompiledClassHash::ZERO,
                signature: tx.signature,
                nonce: Felt::ZERO,
                contract_class,
                paymaster_data: vec![],
                tip: 0.into(),
                account_deployment_data: vec![],
                resource_bounds: ResourceBoundsMapping::L1Gas(L1GasResourceBoundsMapping {
                    l1_gas: ResourceBounds { max_amount: 0, max_price_per_unit: tx.max_fee },
                    l2_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
                }),
                fee_data_availability_mode: DataAvailabilityMode::L1,
                nonce_data_availability_mode: DataAvailabilityMode::L1,
                is_query: false,
            },
            DeclareTx::V1(tx) => BroadcastedDeclareTx {
                sender_address: tx.sender_address,
                compiled_class_hash: CompiledClassHash::ZERO,
                signature: tx.signature,
                nonce: tx.nonce,
                contract_class,
                paymaster_data: vec![],
                tip: 0.into(),
                account_deployment_data: vec![],
                resource_bounds: ResourceBoundsMapping::L1Gas(L1GasResourceBoundsMapping {
                    l1_gas: ResourceBounds { max_amount: 0, max_price_per_unit: tx.max_fee },
                    l2_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
                }),
                fee_data_availability_mode: DataAvailabilityMode::L1,
                nonce_data_availability_mode: DataAvailabilityMode::L1,
                is_query: false,
            },
            DeclareTx::V2(tx) => BroadcastedDeclareTx {
                sender_address: tx.sender_address,
                compiled_class_hash: tx.compiled_class_hash,
                signature: tx.signature,
                nonce: tx.nonce,
                contract_class,
                paymaster_data: vec![],
                tip: 0.into(),
                account_deployment_data: vec![],
                resource_bounds: ResourceBoundsMapping::L1Gas(L1GasResourceBoundsMapping {
                    l1_gas: ResourceBounds { max_amount: 0, max_price_per_unit: tx.max_fee },
                    l2_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
                }),
                fee_data_availability_mode: DataAvailabilityMode::L1,
                nonce_data_availability_mode: DataAvailabilityMode::L1,
                is_query: false,
            },
            DeclareTx::V3(tx) => BroadcastedDeclareTx {
                sender_address: tx.sender_address,
                compiled_class_hash: tx.compiled_class_hash,
                signature: tx.signature,
                nonce: tx.nonce,
                contract_class,
                paymaster_data: tx.paymaster_data,
                tip: tx.tip.into(),
                account_deployment_data: tx.account_deployment_data,
                resource_bounds: tx.resource_bounds,
                fee_data_availability_mode: tx.fee_data_availability_mode,
                nonce_data_availability_mode: tx.nonce_data_availability_mode,
                is_query: false,
            },
        }
    }
}

impl From<DeployAccountTx> for BroadcastedDeployAccountTx {
    fn from(tx: DeployAccountTx) -> Self {
        match tx {
            DeployAccountTx::V1(tx) => tx.into(),
            DeployAccountTx::V3(tx) => tx.into(),
        }
    }
}

impl From<DeployAccountTxV1> for BroadcastedDeployAccountTx {
    fn from(tx: DeployAccountTxV1) -> Self {
        BroadcastedDeployAccountTx {
            signature: tx.signature,
            nonce: tx.nonce,
            contract_address_salt: tx.contract_address_salt,
            constructor_calldata: tx.constructor_calldata,
            class_hash: tx.class_hash,
            paymaster_data: vec![],
            tip: 0.into(),
            resource_bounds: ResourceBoundsMapping::L1Gas(L1GasResourceBoundsMapping {
                l1_gas: ResourceBounds { max_amount: 0, max_price_per_unit: tx.max_fee },
                l2_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
            }),
            fee_data_availability_mode: DataAvailabilityMode::L1,
            nonce_data_availability_mode: DataAvailabilityMode::L1,
            is_query: false,
        }
    }
}

impl From<DeployAccountTxV3> for BroadcastedDeployAccountTx {
    fn from(tx: DeployAccountTxV3) -> Self {
        BroadcastedDeployAccountTx {
            signature: tx.signature,
            nonce: tx.nonce,
            contract_address_salt: tx.contract_address_salt,
            constructor_calldata: tx.constructor_calldata,
            class_hash: tx.class_hash,
            paymaster_data: tx.paymaster_data,
            tip: tx.tip.into(),
            resource_bounds: tx.resource_bounds,
            fee_data_availability_mode: tx.fee_data_availability_mode,
            nonce_data_availability_mode: tx.nonce_data_availability_mode,
            is_query: false,
        }
    }
}

impl From<ExecutableTx> for BroadcastedTx {
    fn from(tx: ExecutableTx) -> Self {
        match tx {
            ExecutableTx::Invoke(tx) => BroadcastedTx::Invoke(tx.into()),
            ExecutableTx::Declare(tx) => BroadcastedTx::Declare(tx.into()),
            ExecutableTx::DeployAccount(tx) => BroadcastedTx::DeployAccount(tx.into()),
            ExecutableTx::L1Handler(_) => unimplemented!(),
        }
    }
}

/// A broadcasted transaction.
#[derive(Debug, Clone, Serialize)]
pub struct BroadcastedTxWithChainId {
    pub tx: BroadcastedTx,
    pub chain: ChainId,
}

impl BroadcastedTxWithChainId {
    /// Compute the hash of the transaction.
    pub fn calculate_hash(&self) -> TxHash {
        let is_query = self.tx.is_query();
        match &self.tx {
            BroadcastedTx::Invoke(tx) => {
                let invoke_tx = InvokeTx::V3(InvokeTxV3 {
                    chain_id: self.chain,
                    tip: tx.tip.into(),
                    nonce: tx.nonce,
                    calldata: tx.calldata.clone(),
                    signature: tx.signature.clone(),
                    sender_address: tx.sender_address,
                    paymaster_data: tx.paymaster_data.clone(),
                    resource_bounds: tx.resource_bounds.clone(),
                    account_deployment_data: tx.account_deployment_data.clone(),
                    fee_data_availability_mode: tx.fee_data_availability_mode,
                    nonce_data_availability_mode: tx.nonce_data_availability_mode,
                });
                invoke_tx.calculate_hash(is_query)
            }

            BroadcastedTx::Declare(tx) => {
                let declare_tx = DeclareTx::V3(DeclareTxV3 {
                    chain_id: self.chain,
                    class_hash: tx.contract_class.hash(),
                    tip: tx.tip.into(),
                    nonce: tx.nonce,
                    signature: tx.signature.clone(),
                    paymaster_data: tx.paymaster_data.clone(),
                    sender_address: tx.sender_address,
                    resource_bounds: tx.resource_bounds.clone(),
                    compiled_class_hash: tx.compiled_class_hash,
                    account_deployment_data: tx.account_deployment_data.clone(),
                    fee_data_availability_mode: tx.fee_data_availability_mode,
                    nonce_data_availability_mode: tx.nonce_data_availability_mode,
                });
                declare_tx.calculate_hash(is_query)
            }

            BroadcastedTx::DeployAccount(tx) => {
                let contract_address = tx.contract_address();
                let deploy_account_tx = DeployAccountTx::V3(DeployAccountTxV3 {
                    chain_id: self.chain,
                    tip: tx.tip.into(),
                    nonce: tx.nonce,
                    contract_address,
                    signature: tx.signature.clone(),
                    class_hash: tx.class_hash,
                    paymaster_data: tx.paymaster_data.clone(),
                    contract_address_salt: tx.contract_address_salt,
                    constructor_calldata: tx.constructor_calldata.clone(),
                    resource_bounds: tx.resource_bounds.clone(),
                    fee_data_availability_mode: tx.fee_data_availability_mode,
                    nonce_data_availability_mode: tx.nonce_data_availability_mode,
                });
                deploy_account_tx.calculate_hash(is_query)
            }
        }
    }
}

impl PoolTransaction for BroadcastedTxWithChainId {
    fn hash(&self) -> TxHash {
        self.calculate_hash()
    }

    fn nonce(&self) -> Nonce {
        match &self.tx {
            BroadcastedTx::Invoke(tx) => tx.nonce,
            BroadcastedTx::Declare(tx) => tx.nonce,
            BroadcastedTx::DeployAccount(tx) => tx.nonce,
        }
    }

    fn sender(&self) -> ContractAddress {
        match &self.tx {
            BroadcastedTx::Invoke(tx) => tx.sender_address,
            BroadcastedTx::Declare(tx) => tx.sender_address,
            BroadcastedTx::DeployAccount(tx) => tx.contract_address(),
        }
    }

    fn max_fee(&self) -> u128 {
        // V3 transactions don't have max_fee, they use resource bounds instead
        0
    }

    fn tip(&self) -> u64 {
        match &self.tx {
            BroadcastedTx::Invoke(tx) => tx.tip.into(),
            BroadcastedTx::Declare(tx) => tx.tip.into(),
            BroadcastedTx::DeployAccount(tx) => tx.tip.into(),
        }
    }
}

/// A broadcasted transaction.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum BroadcastedTx {
    Invoke(BroadcastedInvokeTx),
    Declare(BroadcastedDeclareTx),
    DeployAccount(BroadcastedDeployAccountTx),
}

impl BroadcastedTx {
    pub fn is_query(&self) -> bool {
        match self {
            BroadcastedTx::Invoke(tx) => tx.is_query(),
            BroadcastedTx::Declare(tx) => tx.is_query(),
            BroadcastedTx::DeployAccount(tx) => tx.is_query(),
        }
    }
}

/// A broadcasted `INVOKE` transaction.
#[derive(Debug, Clone)]
pub struct BroadcastedInvokeTx {
    /// The transaction sender.
    ///
    /// Address of the account where the transaction will be executed from.
    pub sender_address: ContractAddress,
    /// Function calldata.
    ///
    /// The data expected by the account's `execute` function (in most usecases, this includes the
    /// called contract address and a function selector).
    pub calldata: Vec<Felt>,
    /// The sender's signature.
    pub signature: Vec<Felt>,
    /// The transaction nonce
    ///
    /// The nonce is used to prevent replay attacks and ensure that each transaction is unique. For
    /// a transaction to be valid for execution, the nonce must be equal to the account's
    /// current nonce.
    pub nonce: Felt,
    /// Data needed to allow the paymaster to pay for the transaction in native tokens.
    pub paymaster_data: Vec<Felt>,
    /// The tip for the transaction.
    pub tip: Tip,
    /// Data needed to deploy the account contract from which this tx will be initiated.
    pub account_deployment_data: Vec<Felt>,
    /// Resource bounds for the transaction execution.
    pub resource_bounds: ResourceBoundsMapping,
    /// The storage domain of the account's balance from which fee will be charged.
    pub fee_data_availability_mode: DataAvailabilityMode,
    /// The storage domain of the account's nonce (an account has a nonce per DA mode).
    pub nonce_data_availability_mode: DataAvailabilityMode,
    /// An internal flag indicating whether this transaction is a query.
    pub is_query: bool,
}

impl BroadcastedInvokeTx {
    pub fn is_query(&self) -> bool {
        self.is_query
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BroadcastedDeclareTxError {
    #[error("failed to compute class hash: {0}")]
    ComputeClassHash(#[from] ComputeClassHashError),
    #[error("invalid contract class: {0}")]
    InvalidContractClass(#[from] crate::class::ConversionError),
}

/// A broadcasted `DECLARE` transaction.
#[derive(Debug, Clone)]
pub struct BroadcastedDeclareTx {
    /// The transaction sender.
    ///
    /// Address of the account where the transaction will be executed from.
    pub sender_address: ContractAddress,
    pub compiled_class_hash: CompiledClassHash,
    /// The sender's signature.
    pub signature: Vec<Felt>,
    /// The transaction nonce
    ///
    /// The nonce is used to prevent replay attacks and ensure that each transaction is unique. For
    /// a transaction to be valid for execution, the nonce must be equal to the account's current
    /// nonce.
    pub nonce: Nonce,
    pub contract_class: Arc<SierraClass>,
    /// Data needed to allow the paymaster to pay for the transaction in native tokens.
    pub paymaster_data: Vec<Felt>,
    /// The tip for the transaction.
    pub tip: Tip,
    /// Data needed to deploy the account contract from which this tx will be initiated.
    pub account_deployment_data: Vec<Felt>,
    /// Resource bounds for the transaction execution.
    pub resource_bounds: ResourceBoundsMapping,
    /// The storage domain of the account's balance from which fee will be charged.
    pub fee_data_availability_mode: DataAvailabilityMode,
    /// The storage domain of the account's nonce (an account has a nonce per DA mode).
    pub nonce_data_availability_mode: DataAvailabilityMode,
    /// An internal flag indicating whether this transaction is a query.
    pub is_query: bool,
}

impl BroadcastedDeclareTx {
    pub fn is_query(&self) -> bool {
        self.is_query
    }
}

/// A broadcasted `DEPLOY_ACCOUNT` transaction.
#[derive(Debug, Clone)]
pub struct BroadcastedDeployAccountTx {
    /// The sender's signature.
    pub signature: Vec<Felt>,
    /// The transaction nonce
    ///
    /// The nonce is used to prevent replay attacks and ensure that each transaction is unique. For
    /// a transaction to be valid for execution, the nonce must be equal to the account's current
    /// nonce.
    pub nonce: Nonce,
    pub contract_address_salt: Felt,
    pub constructor_calldata: Vec<Felt>,
    /// The hash of the class that will be used by the account.
    pub class_hash: ClassHash,
    pub paymaster_data: Vec<Felt>,
    /// The tip for the transaction.
    pub tip: Tip,
    /// Resource bounds for the transaction execution.
    pub resource_bounds: ResourceBoundsMapping,
    /// The storage domain of the account's balance from which fee will be charged.
    pub fee_data_availability_mode: DataAvailabilityMode,
    /// The storage domain of the account's nonce (an account has a nonce per DA mode).
    pub nonce_data_availability_mode: DataAvailabilityMode,
    /// An internal flag indicating whether this transaction is a query.
    pub is_query: bool,
}

impl BroadcastedDeployAccountTx {
    pub fn is_query(&self) -> bool {
        self.is_query
    }

    /// Get the address of the account contract that will be deployed.
    pub fn contract_address(&self) -> ContractAddress {
        get_contract_address(
            self.contract_address_salt,
            self.class_hash,
            &self.constructor_calldata,
            Felt::ZERO,
        )
        .into()
    }
}

/// Response for broadcasting an `INVOKE` transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddInvokeTransactionResponse {
    /// The hash of the invoke transaction
    pub transaction_hash: TxHash,
}

/// Response for broadcasting a `DECLARE` transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddDeclareTransactionResponse {
    /// The hash of the declare transaction
    pub transaction_hash: TxHash,
    /// The hash of the declared class
    pub class_hash: ClassHash,
}

/// Response for broadcasting a `DEPLOY_ACCOUNT` transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddDeployAccountTransactionResponse {
    /// The hash of the deploy transaction
    pub transaction_hash: TxHash,
    /// The address of the new contract
    pub contract_address: ContractAddress,
}

impl<'de> Deserialize<'de> for BroadcastedTx {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        UntypedBroadcastedTx::deserialize(deserializer)?.typed().map_err(de::Error::custom)
    }
}

impl serde::Serialize for BroadcastedInvokeTx {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        UntypedBroadcastedTx::from(self.clone()).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for BroadcastedInvokeTx {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        UntypedBroadcastedTx::deserialize(deserializer)?
            .try_into_invoke()
            .map_err(de::Error::custom)
    }
}

impl serde::Serialize for BroadcastedDeclareTx {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        UntypedBroadcastedTx::from(self.clone()).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for BroadcastedDeclareTx {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        UntypedBroadcastedTx::deserialize(deserializer)?
            .try_into_declare()
            .map_err(de::Error::custom)
    }
}

impl serde::Serialize for BroadcastedDeployAccountTx {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        UntypedBroadcastedTx::from(self.clone()).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for BroadcastedDeployAccountTx {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        UntypedBroadcastedTx::deserialize(deserializer)?
            .try_into_deploy_account()
            .map_err(de::Error::custom)
    }
}

impl From<BroadcastedTxWithChainId> for ExecutableTx {
    fn from(value: BroadcastedTxWithChainId) -> Self {
        match value.tx {
            BroadcastedTx::Invoke(tx) => {
                let invoke_tx = InvokeTx::V3(InvokeTxV3 {
                    chain_id: value.chain,
                    tip: tx.tip.into(),
                    nonce: tx.nonce,
                    calldata: tx.calldata,
                    signature: tx.signature,
                    sender_address: tx.sender_address,
                    paymaster_data: tx.paymaster_data,
                    resource_bounds: tx.resource_bounds,
                    account_deployment_data: tx.account_deployment_data,
                    fee_data_availability_mode: tx.fee_data_availability_mode,
                    nonce_data_availability_mode: tx.nonce_data_availability_mode,
                });

                ExecutableTx::Invoke(invoke_tx)
            }

            BroadcastedTx::Declare(tx) => {
                let class_hash = tx.contract_class.hash();

                let rpc_class = Arc::unwrap_or_clone(tx.contract_class);
                let class = ContractClass::Class(SierraContractClass::from(rpc_class));

                let declare_tx = DeclareTx::V3(DeclareTxV3 {
                    chain_id: value.chain,
                    class_hash,
                    tip: tx.tip.into(),
                    nonce: tx.nonce,
                    signature: tx.signature,
                    paymaster_data: tx.paymaster_data,
                    sender_address: tx.sender_address,
                    resource_bounds: tx.resource_bounds,
                    compiled_class_hash: tx.compiled_class_hash,
                    account_deployment_data: tx.account_deployment_data,
                    fee_data_availability_mode: tx.fee_data_availability_mode,
                    nonce_data_availability_mode: tx.nonce_data_availability_mode,
                });

                ExecutableTx::Declare(DeclareTxWithClass::new(declare_tx, class))
            }

            BroadcastedTx::DeployAccount(tx) => {
                let contract_address = tx.contract_address();

                let deploy_account_tx = DeployAccountTx::V3(DeployAccountTxV3 {
                    chain_id: value.chain,
                    tip: tx.tip.into(),
                    nonce: tx.nonce,
                    contract_address,
                    signature: tx.signature,
                    class_hash: tx.class_hash,
                    paymaster_data: tx.paymaster_data,
                    contract_address_salt: tx.contract_address_salt,
                    constructor_calldata: tx.constructor_calldata,
                    resource_bounds: tx.resource_bounds,
                    fee_data_availability_mode: tx.fee_data_availability_mode,
                    nonce_data_availability_mode: tx.nonce_data_availability_mode,
                });

                ExecutableTx::DeployAccount(deploy_account_tx)
            }
        }
    }
}

impl From<BroadcastedTxWithChainId> for ExecutableTxWithHash {
    fn from(value: BroadcastedTxWithChainId) -> Self {
        let is_query = value.tx.is_query();
        let tx = ExecutableTx::from(value);
        ExecutableTxWithHash::new_query(tx, is_query)
    }
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
        AddDeclareTransactionResponse, AddDeployAccountTransactionResponse,
        AddInvokeTransactionResponse, BroadcastedTx, UntypedBroadcastedTxError,
    };

    #[test]
    fn untyped_invoke_tx_serde() {
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

        let untyped: UntypedBroadcastedTx = serde_json::from_value(json.clone()).unwrap();

        assert_eq!(untyped.version, Felt::THREE);
        assert_eq!(untyped.r#type, TxType::Invoke);

        // Common fields
        assert_eq!(untyped.tip, Tip::from(0x0));
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
        assert_matches!(&typed, BroadcastedTx::Invoke(tx) => {
            // common fields
            assert_eq!(tx.tip, untyped.tip);
            assert_eq!(tx.nonce, untyped.nonce);
            assert_eq!(tx.signature, untyped.signature);
            assert_eq!(tx.paymaster_data, untyped.paymaster_data);
            assert_eq!(tx.fee_data_availability_mode, untyped.fee_data_availability_mode);
            assert_eq!(tx.nonce_data_availability_mode, untyped.nonce_data_availability_mode);

            // tx specific fields
            assert_eq!(tx.calldata,  untyped.calldata.clone().unwrap());
            assert_eq!(tx.sender_address, untyped.sender_address.unwrap());
        });

        let new_untyped = UntypedBroadcastedTx::from(typed.clone());
        similar_asserts::assert_eq!(untyped, new_untyped);

        // Serializing typed transaction should yield the same JSON as the original
        let new_json = serde_json::to_value(typed).unwrap();
        similar_asserts::assert_eq!(json, new_json);
    }

    #[test]
    fn untyped_declare_tx_serde() {
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
                "abi": "[]"
            }
        });

        let untyped: UntypedBroadcastedTx = serde_json::from_value(json.clone()).unwrap();

        assert_eq!(untyped.version, Felt::THREE);
        assert_eq!(untyped.r#type, TxType::Declare);

        // Common fields
        assert_eq!(untyped.tip, Tip::from(0x0));
        assert_eq!(untyped.nonce, felt!("0x1"));
        assert_eq!(untyped.paymaster_data, vec![]);
        assert_eq!(untyped.fee_data_availability_mode, DataAvailabilityMode::L1);
        assert_eq!(untyped.nonce_data_availability_mode, DataAvailabilityMode::L1);
        assert_matches!(&untyped.resource_bounds, ResourceBoundsMapping::L1Gas(bounds) => {
            assert_eq!(bounds.l1_gas.max_amount, 0x100);
            assert_eq!(bounds.l1_gas.max_price_per_unit, 0x200);
            assert_eq!(bounds.l2_gas.max_amount, 0x0);
            assert_eq!(bounds.l2_gas.max_price_per_unit, 0x0);
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
        assert_matches!(&typed, BroadcastedTx::Declare(tx) => {
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

        let new_untyped = UntypedBroadcastedTx::from(typed.clone());
        similar_asserts::assert_eq!(untyped, new_untyped);

        // Serializing typed transaction should yield the same JSON as the original
        let new_json = serde_json::to_value(typed).unwrap();
        similar_asserts::assert_eq!(json, new_json);
    }

    #[test]
    fn untyped_deploy_account_tx_serde() {
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
                    "max_amount": "0x123",
                    "max_price_per_unit": "0x1337"
                }
            },
            "nonce_data_availability_mode": "L1",
            "fee_data_availability_mode": "L1",
            "nonce": "0x0",
            "contract_address_salt": "0xabc",
            "constructor_calldata": ["0x1", "0x2"],
            "class_hash": "0xdef"
        });

        let untyped: UntypedBroadcastedTx = serde_json::from_value(json.clone()).unwrap();

        assert_eq!(untyped.version, Felt::THREE);
        assert_eq!(untyped.r#type, TxType::DeployAccount);

        // Common fields
        assert_eq!(untyped.tip, Tip::from(0x1));
        assert_eq!(untyped.nonce, felt!("0x0"));
        assert_eq!(untyped.paymaster_data, vec![]);
        assert_eq!(untyped.signature, vec![felt!("0x1"), felt!("0x2"), felt!("0x3")]);
        assert_eq!(untyped.fee_data_availability_mode, DataAvailabilityMode::L1);
        assert_eq!(untyped.nonce_data_availability_mode, DataAvailabilityMode::L1);
        assert_matches!(&untyped.resource_bounds, ResourceBoundsMapping::L1Gas(bounds) => {
            assert_eq!(bounds.l1_gas.max_amount, 0x100);
            assert_eq!(bounds.l1_gas.max_price_per_unit, 0x200);
            assert_eq!(bounds.l2_gas.max_amount, 0x123);
            assert_eq!(bounds.l2_gas.max_price_per_unit, 0x1337);
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
        assert_matches!(&typed, BroadcastedTx::DeployAccount(tx) => {
            // common fields
            assert_eq!(tx.tip, untyped.tip);
            assert_eq!(tx.nonce, untyped.nonce);
            assert_eq!(tx.signature, untyped.signature);
            assert_eq!(tx.paymaster_data, untyped.paymaster_data);
            assert_eq!(tx.fee_data_availability_mode, untyped.fee_data_availability_mode);
            assert_eq!(tx.nonce_data_availability_mode, untyped.nonce_data_availability_mode);

            // tx specific fields
            assert_eq!(tx.class_hash, untyped.class_hash.unwrap());
            assert_eq!(tx.contract_address_salt,  untyped.contract_address_salt.unwrap());
            assert_eq!(tx.constructor_calldata, untyped.constructor_calldata.clone().unwrap());
        });

        let new_untyped = UntypedBroadcastedTx::from(typed.clone());
        similar_asserts::assert_eq!(untyped, new_untyped);

        // Serializing typed transaction should yield the same JSON as the original
        let new_json = serde_json::to_value(typed).unwrap();
        similar_asserts::assert_eq!(json, new_json);
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
    fn broadcasted_tx_enum_serde() {
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

        let tx: BroadcastedTx = serde_json::from_value(json.clone()).unwrap();
        assert_matches!(tx, BroadcastedTx::Invoke(_));

        let new_json = serde_json::to_value(tx).unwrap();
        similar_asserts::assert_eq!(json, new_json);
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
    fn response_types_serde() {
        let invoke_result = AddInvokeTransactionResponse { transaction_hash: felt!("0x123") };

        let expected = json!({
            "transaction_hash": "0x123"
        });

        let actual = serde_json::to_value(&invoke_result).unwrap();
        similar_asserts::assert_eq!(actual, expected);

        let declare_result = AddDeclareTransactionResponse {
            transaction_hash: felt!("0x456"),
            class_hash: felt!("0x789"),
        };

        let expected = json!({
            "transaction_hash": "0x456",
            "class_hash": "0x789"
        });

        let actual = serde_json::to_value(&declare_result).unwrap();
        similar_asserts::assert_eq!(actual, expected);

        let deploy_result = AddDeployAccountTransactionResponse {
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

    #[test]
    fn broadcasted_tx_with_chain_id_calculate_hash() {
        use katana_primitives::chain::ChainId;

        // Test with an Invoke transaction
        let invoke_json = json!({
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
            "calldata": ["0x1", "0x2"],
            "nonce": "0x0",
            "account_deployment_data": []
        });

        let invoke_tx: BroadcastedTx = serde_json::from_value(invoke_json).unwrap();
        let tx_with_chain = BroadcastedTxWithChainId { tx: invoke_tx, chain: ChainId::SEPOLIA };

        // The hash should be computed successfully
        let hash = tx_with_chain.calculate_hash();
        assert_ne!(hash, Felt::ZERO);

        // Test with a DeployAccount transaction
        let deploy_account_json = json!({
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
                    "max_amount": "0x123",
                    "max_price_per_unit": "0x1337"
                }
            },
            "nonce_data_availability_mode": "L1",
            "fee_data_availability_mode": "L1",
            "nonce": "0x0",
            "contract_address_salt": "0xabc",
            "constructor_calldata": ["0x1", "0x2"],
            "class_hash": "0xdef"
        });

        let deploy_account_tx: BroadcastedTx = serde_json::from_value(deploy_account_json).unwrap();
        let tx_with_chain =
            BroadcastedTxWithChainId { tx: deploy_account_tx, chain: ChainId::SEPOLIA };

        // The hash should be computed successfully
        let hash = tx_with_chain.calculate_hash();
        assert_ne!(hash, Felt::ZERO);
    }

    // This will currently fail because the l2 gas on legacy resource bounds format are not
    // preserved throughout the serialization process.
    #[test]
    #[ignore]
    fn legacy_resource_bounds_with_nonzero_l2_gas() {
        let json = json!({
          "account_deployment_data": [],
          "calldata": [
            "0x4",
            "0xcf"
          ],
          "fee_data_availability_mode": "L1",
          "nonce": "0x4c7",
          "nonce_data_availability_mode": "L1",
          "paymaster_data": [],
          "resource_bounds": {
            "l1_gas": {
              "max_amount": "0x360",
              "max_price_per_unit": "0x1626"
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

        assert_eq!(json["resource_bounds"]["l2_gas"]["max_amount"], "0xc1e300");
        assert_eq!(json["resource_bounds"]["l2_gas"]["max_price_per_unit"], "0x4e575794");

        let untyped: UntypedBroadcastedTx = serde_json::from_value(json.clone()).unwrap();
        let new_json = serde_json::to_value(&untyped).unwrap();

        assert_eq!(new_json["resource_bounds"]["l2_gas"]["max_amount"], "0xc1e300");
        assert_eq!(new_json["resource_bounds"]["l2_gas"]["max_price_per_unit"], "0x4e575794");
    }
}
