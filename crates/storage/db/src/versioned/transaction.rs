use katana_primitives::chain::ChainId;
use katana_primitives::class::{ClassHash, CompiledClassHash};
use katana_primitives::contract::{ContractAddress, Nonce};
use katana_primitives::da::DataAvailabilityMode;
use katana_primitives::fee::{ResourceBounds, ResourceBoundsMapping};
use katana_primitives::transaction::{
    DeclareTxV0, DeclareTxV1, DeclareTxV2, DeployAccountTxV1, DeployTx, L1HandlerTx, Tx, TxType,
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
    V0(katana_primitives::transaction::InvokeTxV0),
    V1(katana_primitives::transaction::InvokeTxV1),
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

impl From<ResourceBoundsMappingV6> for ResourceBoundsMapping {
    fn from(v6: ResourceBoundsMappingV6) -> Self {
        ResourceBoundsMapping {
            l1_gas: v6.l1_gas,
            l2_gas: v6.l2_gas,
            l1_data_gas: ResourceBounds::default(),
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
                    UntaggedVersionedTx::V5(tx) => Self::V5(tx),
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
