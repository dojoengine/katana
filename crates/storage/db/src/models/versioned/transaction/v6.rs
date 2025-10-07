//! IMPORTANT: The ordering for enumerable types should not be modified as this is the order that
//! has been defined in database version 6. Modifying the order will break compatibility with the
//! version.

use katana_primitives::fee::{self};
use katana_primitives::{chain, class, contract, da, transaction, Felt};
use serde::{Deserialize, Serialize};

#[repr(u8)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(::arbitrary::Arbitrary))]
pub enum Tx {
    Invoke(InvokeTx) = 0,
    Declare(DeclareTx),
    L1Handler(transaction::L1HandlerTx),
    DeployAccount(DeployAccountTx),
    Deploy(transaction::DeployTx),
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(test, derive(::arbitrary::Arbitrary))]
pub struct ResourceBoundsMapping {
    pub l1_gas: fee::ResourceBounds,
    pub l2_gas: fee::ResourceBounds,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(::arbitrary::Arbitrary))]
pub struct InvokeTxV3 {
    pub chain_id: chain::ChainId,
    pub sender_address: contract::ContractAddress,
    pub nonce: Felt,
    pub calldata: Vec<Felt>,
    pub signature: Vec<Felt>,
    pub resource_bounds: ResourceBoundsMapping,
    pub tip: u64,
    pub paymaster_data: Vec<Felt>,
    pub account_deployment_data: Vec<Felt>,
    pub nonce_data_availability_mode: da::DataAvailabilityMode,
    pub fee_data_availability_mode: da::DataAvailabilityMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(::arbitrary::Arbitrary))]
pub struct DeclareTxV3 {
    pub chain_id: chain::ChainId,
    pub sender_address: contract::ContractAddress,
    pub nonce: Felt,
    pub signature: Vec<Felt>,
    pub class_hash: class::ClassHash,
    pub compiled_class_hash: class::CompiledClassHash,
    pub resource_bounds: ResourceBoundsMapping,
    pub tip: u64,
    pub paymaster_data: Vec<Felt>,
    pub account_deployment_data: Vec<Felt>,
    pub nonce_data_availability_mode: da::DataAvailabilityMode,
    pub fee_data_availability_mode: da::DataAvailabilityMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(::arbitrary::Arbitrary))]
pub struct DeployAccountTxV3 {
    pub chain_id: chain::ChainId,
    pub nonce: contract::Nonce,
    pub signature: Vec<Felt>,
    pub class_hash: class::ClassHash,
    pub contract_address: contract::ContractAddress,
    pub contract_address_salt: Felt,
    pub constructor_calldata: Vec<Felt>,
    pub resource_bounds: ResourceBoundsMapping,
    pub tip: u64,
    pub paymaster_data: Vec<Felt>,
    pub nonce_data_availability_mode: da::DataAvailabilityMode,
    pub fee_data_availability_mode: da::DataAvailabilityMode,
}

#[repr(u8)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(::arbitrary::Arbitrary))]
pub enum InvokeTx {
    V0(transaction::InvokeTxV0) = 0,
    V1(transaction::InvokeTxV1),
    V3(InvokeTxV3),
}

#[repr(u8)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(::arbitrary::Arbitrary))]
pub enum DeclareTx {
    V1(transaction::DeclareTxV1) = 0,
    V2(transaction::DeclareTxV2) = 1,
    V3(DeclareTxV3) = 2,
    V0(transaction::DeclareTxV0) = 3,
}

#[repr(u8)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(::arbitrary::Arbitrary))]
pub enum DeployAccountTx {
    V1(transaction::DeployAccountTxV1) = 0,
    V3(DeployAccountTxV3),
}

impl From<ResourceBoundsMapping> for fee::ResourceBoundsMapping {
    fn from(v6: ResourceBoundsMapping) -> Self {
        Self::L1Gas(fee::L1GasResourceBoundsMapping { l1_gas: v6.l1_gas, l2_gas: v6.l2_gas })
    }
}

impl From<InvokeTxV3> for transaction::InvokeTxV3 {
    fn from(v6: InvokeTxV3) -> Self {
        Self {
            chain_id: v6.chain_id,
            sender_address: v6.sender_address,
            nonce: v6.nonce,
            calldata: v6.calldata,
            signature: v6.signature,
            resource_bounds: v6.resource_bounds.into(),
            tip: v6.tip,
            paymaster_data: v6.paymaster_data,
            account_deployment_data: v6.account_deployment_data,
            nonce_data_availability_mode: v6.nonce_data_availability_mode,
            fee_data_availability_mode: v6.fee_data_availability_mode,
        }
    }
}

impl From<DeclareTxV3> for transaction::DeclareTxV3 {
    fn from(v6: DeclareTxV3) -> Self {
        Self {
            chain_id: v6.chain_id,
            sender_address: v6.sender_address,
            nonce: v6.nonce,
            signature: v6.signature,
            class_hash: v6.class_hash,
            compiled_class_hash: v6.compiled_class_hash,
            resource_bounds: v6.resource_bounds.into(),
            tip: v6.tip,
            paymaster_data: v6.paymaster_data,
            account_deployment_data: v6.account_deployment_data,
            nonce_data_availability_mode: v6.nonce_data_availability_mode,
            fee_data_availability_mode: v6.fee_data_availability_mode,
        }
    }
}

impl From<DeployAccountTxV3> for transaction::DeployAccountTxV3 {
    fn from(v6: DeployAccountTxV3) -> Self {
        Self {
            chain_id: v6.chain_id,
            nonce: v6.nonce,
            signature: v6.signature,
            class_hash: v6.class_hash,
            contract_address: v6.contract_address,
            contract_address_salt: v6.contract_address_salt,
            constructor_calldata: v6.constructor_calldata,
            resource_bounds: v6.resource_bounds.into(),
            tip: v6.tip,
            paymaster_data: v6.paymaster_data,
            nonce_data_availability_mode: v6.nonce_data_availability_mode,
            fee_data_availability_mode: v6.fee_data_availability_mode,
        }
    }
}

impl From<InvokeTx> for transaction::InvokeTx {
    fn from(v6: InvokeTx) -> Self {
        match v6 {
            InvokeTx::V0(tx) => transaction::InvokeTx::V0(tx),
            InvokeTx::V1(tx) => transaction::InvokeTx::V1(tx),
            InvokeTx::V3(tx) => transaction::InvokeTx::V3(tx.into()),
        }
    }
}

impl From<DeclareTx> for transaction::DeclareTx {
    fn from(v6: DeclareTx) -> Self {
        match v6 {
            DeclareTx::V0(tx) => transaction::DeclareTx::V0(tx),
            DeclareTx::V1(tx) => transaction::DeclareTx::V1(tx),
            DeclareTx::V2(tx) => transaction::DeclareTx::V2(tx),
            DeclareTx::V3(tx) => transaction::DeclareTx::V3(tx.into()),
        }
    }
}

impl From<DeployAccountTx> for transaction::DeployAccountTx {
    fn from(v6: DeployAccountTx) -> Self {
        match v6 {
            DeployAccountTx::V1(tx) => transaction::DeployAccountTx::V1(tx),
            DeployAccountTx::V3(tx) => transaction::DeployAccountTx::V3(tx.into()),
        }
    }
}

impl From<Tx> for transaction::Tx {
    fn from(v6: Tx) -> Self {
        match v6 {
            Tx::Invoke(tx) => transaction::Tx::Invoke(tx.into()),
            Tx::Declare(tx) => transaction::Tx::Declare(tx.into()),
            Tx::L1Handler(tx) => transaction::Tx::L1Handler(tx),
            Tx::DeployAccount(tx) => transaction::Tx::DeployAccount(tx.into()),
            Tx::Deploy(tx) => transaction::Tx::Deploy(tx),
        }
    }
}

#[cfg(test)]
mod tests {
    use katana_primitives::{da, fee, transaction, Felt};

    use super::{InvokeTx, Tx};
    use crate::models::versioned::transaction::v6::{InvokeTxV3, ResourceBoundsMapping};
    use crate::models::versioned::transaction::VersionedTx;

    #[test]
    fn test_versioned_tx_v6_invoke_conversion() {
        let v6_tx = Tx::Invoke(InvokeTx::V0(transaction::InvokeTxV0::default()));
        let versioned = VersionedTx::V6(v6_tx);

        let converted: transaction::Tx = versioned.into();
        if let transaction::Tx::Invoke(transaction::InvokeTx::V0(_)) = converted {
            // Success
        } else {
            panic!("Expected InvokeTx::V0");
        }
    }

    #[test]
    fn test_resource_bounds_mapping_v6_conversion() {
        let v6_mapping = ResourceBoundsMapping {
            l1_gas: fee::ResourceBounds { max_amount: 1000, max_price_per_unit: 100 },
            l2_gas: fee::ResourceBounds { max_amount: 2000, max_price_per_unit: 200 },
        };

        let converted: fee::ResourceBoundsMapping = v6_mapping.into();

        match converted {
            fee::ResourceBoundsMapping::L1Gas(bounds) => {
                assert_eq!(bounds.l1_gas.max_amount, 1000);
                assert_eq!(bounds.l1_gas.max_price_per_unit, 100);
                assert_eq!(bounds.l2_gas.max_amount, 2000);
                assert_eq!(bounds.l2_gas.max_price_per_unit, 200);
            }
            fee::ResourceBoundsMapping::All(..) => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_invoke_tx_v3_v6_conversion() {
        let v6_tx = InvokeTxV3 {
            chain_id: Default::default(),
            sender_address: Default::default(),
            nonce: Felt::ONE,
            calldata: vec![Felt::TWO, Felt::THREE],
            signature: vec![Felt::from(123u32)],
            resource_bounds: ResourceBoundsMapping {
                l1_gas: fee::ResourceBounds { max_amount: 1000, max_price_per_unit: 100 },
                l2_gas: fee::ResourceBounds { max_amount: 2000, max_price_per_unit: 200 },
            },
            tip: 50,
            paymaster_data: vec![],
            account_deployment_data: vec![],
            nonce_data_availability_mode: da::DataAvailabilityMode::L1,
            fee_data_availability_mode: da::DataAvailabilityMode::L1,
        };

        let converted: transaction::InvokeTxV3 = v6_tx.into();
        assert_eq!(converted.nonce, Felt::ONE);
        assert_eq!(converted.calldata, vec![Felt::TWO, Felt::THREE]);
        assert_eq!(converted.signature, vec![Felt::from(123u32)]);
        assert_eq!(converted.tip, 50);

        match converted.resource_bounds {
            fee::ResourceBoundsMapping::L1Gas(bounds) => {
                assert_eq!(bounds.l1_gas.max_amount, 1000);
                assert_eq!(bounds.l1_gas.max_price_per_unit, 100);
                assert_eq!(bounds.l2_gas.max_amount, 2000);
                assert_eq!(bounds.l2_gas.max_price_per_unit, 200);
            }
            fee::ResourceBoundsMapping::All(..) => panic!("wrong variant"),
        }
    }
}
