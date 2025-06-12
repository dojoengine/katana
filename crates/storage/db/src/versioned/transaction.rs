use katana_primitives::chain::ChainId;
use katana_primitives::class::{ClassHash, CompiledClassHash};
use katana_primitives::contract::{ContractAddress, Nonce};
use katana_primitives::da::DataAvailabilityMode;
use katana_primitives::fee::{ResourceBounds, ResourceBoundsMapping};
use katana_primitives::transaction::{
    DeclareTx, DeclareTxV0, DeclareTxV1, DeclareTxV2, DeclareTxV3, DeployAccountTx,
    DeployAccountTxV1, DeployAccountTxV3, DeployTx, InvokeTx, InvokeTxV0, InvokeTxV1, InvokeTxV3,
    L1HandlerTx, Tx, TxType,
};
use katana_primitives::Felt;
use serde::{Deserialize, Serialize};

use crate::codecs::{Compress, Decompress};
use crate::error::CodecError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum VersionedTx {
    V6(TxV6),
    V7(Tx),
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ResourceBoundsMappingV6 {
    #[serde(alias = "L1_GAS")]
    pub l1_gas: ResourceBounds,
    #[serde(alias = "L2_GAS")]
    pub l2_gas: ResourceBounds,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvokeTxV3V6 {
    pub chain_id: ChainId,
    pub sender_address: ContractAddress,
    pub nonce: Felt,
    pub calldata: Vec<Felt>,
    pub signature: Vec<Felt>,
    pub resource_bounds: ResourceBoundsMappingV6,
    pub tip: u64,
    pub paymaster_data: Vec<Felt>,
    pub account_deployment_data: Vec<Felt>,
    pub nonce_data_availability_mode: DataAvailabilityMode,
    pub fee_data_availability_mode: DataAvailabilityMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeclareTxV3V6 {
    pub chain_id: ChainId,
    pub sender_address: ContractAddress,
    pub nonce: Felt,
    pub signature: Vec<Felt>,
    pub class_hash: ClassHash,
    pub compiled_class_hash: CompiledClassHash,
    pub resource_bounds: ResourceBoundsMappingV6,
    pub tip: u64,
    pub paymaster_data: Vec<Felt>,
    pub account_deployment_data: Vec<Felt>,
    pub nonce_data_availability_mode: DataAvailabilityMode,
    pub fee_data_availability_mode: DataAvailabilityMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployAccountTxV3V6 {
    pub chain_id: ChainId,
    pub nonce: Nonce,
    pub signature: Vec<Felt>,
    pub class_hash: ClassHash,
    pub contract_address: ContractAddress,
    pub contract_address_salt: Felt,
    pub constructor_calldata: Vec<Felt>,
    pub resource_bounds: ResourceBoundsMappingV6,
    pub tip: u64,
    pub paymaster_data: Vec<Felt>,
    pub nonce_data_availability_mode: DataAvailabilityMode,
    pub fee_data_availability_mode: DataAvailabilityMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InvokeTxV6 {
    V0(InvokeTxV0),
    V1(InvokeTxV1),
    V3(InvokeTxV3V6),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeclareTxV6 {
    V0(DeclareTxV0),
    V1(DeclareTxV1),
    V2(DeclareTxV2),
    V3(DeclareTxV3V6),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeployAccountTxV6 {
    V1(DeployAccountTxV1),
    V3(DeployAccountTxV3V6),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TxV6 {
    Invoke(InvokeTxV6),
    Declare(DeclareTxV6),
    L1Handler(L1HandlerTx),
    DeployAccount(DeployAccountTxV6),
    Deploy(DeployTx),
}

impl TxV6 {
    pub fn version(&self) -> Felt {
        match self {
            TxV6::Invoke(tx) => match tx {
                InvokeTxV6::V0(_) => Felt::ZERO,
                InvokeTxV6::V1(_) => Felt::ONE,
                InvokeTxV6::V3(_) => Felt::THREE,
            },
            TxV6::Declare(tx) => match tx {
                DeclareTxV6::V0(_) => Felt::ZERO,
                DeclareTxV6::V1(_) => Felt::ONE,
                DeclareTxV6::V2(_) => Felt::TWO,
                DeclareTxV6::V3(_) => Felt::THREE,
            },
            TxV6::L1Handler(tx) => tx.version,
            TxV6::DeployAccount(tx) => match tx {
                DeployAccountTxV6::V1(_) => Felt::ONE,
                DeployAccountTxV6::V3(_) => Felt::THREE,
            },
            TxV6::Deploy(tx) => tx.version,
        }
    }

    pub fn r#type(&self) -> TxType {
        match self {
            Self::Invoke(_) => TxType::Invoke,
            Self::Deploy(_) => TxType::Deploy,
            Self::Declare(_) => TxType::Declare,
            Self::L1Handler(_) => TxType::L1Handler,
            Self::DeployAccount(_) => TxType::DeployAccount,
        }
    }
}

impl From<Tx> for VersionedTx {
    fn from(tx: Tx) -> Self {
        VersionedTx::V7(tx)
    }
}

impl Compress for VersionedTx {
    type Compressed = Vec<u8>;
    fn compress(self) -> Result<Self::Compressed, CodecError> {
        postcard::to_stdvec(&self).map_err(|e| CodecError::Compress(e.to_string()))
    }
}

impl Decompress for VersionedTx {
    fn decompress<B: AsRef<[u8]>>(bytes: B) -> Result<Self, CodecError> {
        let bytes = bytes.as_ref();

        if let Ok(tx) = postcard::from_bytes::<Self>(bytes) {
            return Ok(tx);
        }

        #[derive(Debug, Deserialize)]
        #[serde(untagged)]
        enum UntaggedVersionedTx {
            V6(TxV6),
            V7(Tx),
        }

        impl From<UntaggedVersionedTx> for VersionedTx {
            fn from(versioned: UntaggedVersionedTx) -> Self {
                match versioned {
                    UntaggedVersionedTx::V6(tx) => Self::V6(tx),
                    UntaggedVersionedTx::V7(tx) => Self::V7(tx),
                }
            }
        }

        postcard::from_bytes::<UntaggedVersionedTx>(bytes)
            .map(Self::from)
            .map_err(|e| CodecError::Decompress(e.to_string()))
    }
}

impl From<ResourceBoundsMappingV6> for ResourceBoundsMapping {
    fn from(v6: ResourceBoundsMappingV6) -> Self {
        Self { l1_gas: v6.l1_gas, l2_gas: v6.l2_gas, l1_data_gas: ResourceBounds::default() }
    }
}

impl From<InvokeTxV3V6> for InvokeTxV3 {
    fn from(v6: InvokeTxV3V6) -> Self {
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

impl From<DeclareTxV3V6> for DeclareTxV3 {
    fn from(v6: DeclareTxV3V6) -> Self {
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

impl From<DeployAccountTxV3V6> for DeployAccountTxV3 {
    fn from(v6: DeployAccountTxV3V6) -> Self {
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

impl From<InvokeTxV6> for InvokeTx {
    fn from(v6: InvokeTxV6) -> Self {
        match v6 {
            InvokeTxV6::V0(tx) => InvokeTx::V0(tx),
            InvokeTxV6::V1(tx) => InvokeTx::V1(tx),
            InvokeTxV6::V3(tx) => InvokeTx::V3(tx.into()),
        }
    }
}

impl From<DeclareTxV6> for DeclareTx {
    fn from(v6: DeclareTxV6) -> Self {
        match v6 {
            DeclareTxV6::V0(tx) => DeclareTx::V0(tx),
            DeclareTxV6::V1(tx) => DeclareTx::V1(tx),
            DeclareTxV6::V2(tx) => DeclareTx::V2(tx),
            DeclareTxV6::V3(tx) => DeclareTx::V3(tx.into()),
        }
    }
}

impl From<DeployAccountTxV6> for DeployAccountTx {
    fn from(v6: DeployAccountTxV6) -> Self {
        match v6 {
            DeployAccountTxV6::V1(tx) => DeployAccountTx::V1(tx),
            DeployAccountTxV6::V3(tx) => DeployAccountTx::V3(tx.into()),
        }
    }
}

impl From<TxV6> for Tx {
    fn from(v6: TxV6) -> Self {
        match v6 {
            TxV6::Invoke(tx) => Tx::Invoke(tx.into()),
            TxV6::Declare(tx) => Tx::Declare(tx.into()),
            TxV6::L1Handler(tx) => Tx::L1Handler(tx),
            TxV6::DeployAccount(tx) => Tx::DeployAccount(tx.into()),
            TxV6::Deploy(tx) => Tx::Deploy(tx),
        }
    }
}

impl From<VersionedTx> for Tx {
    fn from(versioned: VersionedTx) -> Self {
        match versioned {
            VersionedTx::V6(tx) => tx.into(),
            VersionedTx::V7(tx) => tx,
        }
    }
}

#[cfg(test)]
mod tests {
    use katana_primitives::fee::ResourceBounds;
    use katana_primitives::transaction::InvokeTxV3;
    use katana_primitives::Felt;

    use super::*;

    #[test]
    fn test_versioned_tx_v7_conversion() {
        let original_tx = Tx::Invoke(InvokeTx::V0(InvokeTxV0::default()));
        let versioned = VersionedTx::V7(original_tx.clone());

        let converted: Tx = versioned.into();
        assert_eq!(converted, original_tx);
    }

    #[test]
    fn test_versioned_tx_v6_invoke_conversion() {
        let v6_tx = TxV6::Invoke(InvokeTxV6::V0(InvokeTxV0::default()));
        let versioned = VersionedTx::V6(v6_tx);

        let converted: Tx = versioned.into();
        if let Tx::Invoke(InvokeTx::V0(_)) = converted {
            // Success
        } else {
            panic!("Expected InvokeTx::V0");
        }
    }

    #[test]
    fn test_resource_bounds_mapping_v6_conversion() {
        let v6_mapping = ResourceBoundsMappingV6 {
            l1_gas: ResourceBounds { max_amount: 1000, max_price_per_unit: 100 },
            l2_gas: ResourceBounds { max_amount: 2000, max_price_per_unit: 200 },
        };

        let converted: ResourceBoundsMapping = v6_mapping.into();
        assert_eq!(converted.l1_gas.max_amount, 1000);
        assert_eq!(converted.l1_gas.max_price_per_unit, 100);
        assert_eq!(converted.l2_gas.max_amount, 2000);
        assert_eq!(converted.l2_gas.max_price_per_unit, 200);
        assert_eq!(converted.l1_data_gas, ResourceBounds::default());
    }

    #[test]
    fn test_invoke_tx_v3_v6_conversion() {
        let v6_tx = InvokeTxV3V6 {
            chain_id: Default::default(),
            sender_address: Default::default(),
            nonce: Felt::ONE,
            calldata: vec![Felt::TWO, Felt::THREE],
            signature: vec![Felt::from(123u32)],
            resource_bounds: ResourceBoundsMappingV6 {
                l1_gas: ResourceBounds { max_amount: 1000, max_price_per_unit: 100 },
                l2_gas: ResourceBounds { max_amount: 2000, max_price_per_unit: 200 },
            },
            tip: 50,
            paymaster_data: vec![],
            account_deployment_data: vec![],
            nonce_data_availability_mode: DataAvailabilityMode::L1,
            fee_data_availability_mode: DataAvailabilityMode::L1,
        };

        let converted: InvokeTxV3 = v6_tx.into();
        assert_eq!(converted.nonce, Felt::ONE);
        assert_eq!(converted.calldata, vec![Felt::TWO, Felt::THREE]);
        assert_eq!(converted.signature, vec![Felt::from(123u32)]);
        assert_eq!(converted.tip, 50);
        assert_eq!(converted.resource_bounds.l1_gas.max_amount, 1000);
        assert_eq!(converted.resource_bounds.l2_gas.max_amount, 2000);
        assert_eq!(converted.resource_bounds.l1_data_gas, ResourceBounds::default());
    }
}
