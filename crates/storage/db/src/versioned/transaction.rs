use katana_primitives::chain::ChainId;
use katana_primitives::class::{ClassHash, CompiledClassHash};
use katana_primitives::contract::{ContractAddress, Nonce};
use katana_primitives::da::DataAvailabilityMode;
use katana_primitives::fee::{ResourceBounds, ResourceBoundsMapping};
use katana_primitives::transaction::{
    DeclareTx, DeployAccountTx, DeployTx, InvokeTx, L1HandlerTx, Tx, TxType,
};
use katana_primitives::Felt;
use serde::{Deserialize, Serialize};

use crate::codecs::{Compress, Decompress};
use crate::error::CodecError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum VersionedTx {
    V5(TxV5),
    V6(TxV6),
    V7(Tx),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TxV5 {
    Invoke(InvokeTx),
    Declare(DeclareTx),
    L1Handler(L1HandlerTx),
    DeployAccount(DeployAccountTx),
    Deploy(DeployTx),
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ResourceBoundsMappingV6 {
    pub l1_gas: ResourceBounds,
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
    V0(katana_primitives::transaction::DeclareTxV0),
    V1(katana_primitives::transaction::DeclareTxV1),
    V2(katana_primitives::transaction::DeclareTxV2),
    V3(DeclareTxV3V6),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeployAccountTxV6 {
    V1(katana_primitives::transaction::DeployAccountTxV1),
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

impl TxV5 {
    pub fn version(&self) -> Felt {
        match self {
            TxV5::Invoke(tx) => match tx {
                InvokeTx::V0(_) => Felt::ZERO,
                InvokeTx::V1(_) => Felt::ONE,
                InvokeTx::V3(_) => Felt::THREE,
            },
            TxV5::Declare(tx) => match tx {
                DeclareTx::V0(_) => Felt::ZERO,
                DeclareTx::V1(_) => Felt::ONE,
                DeclareTx::V2(_) => Felt::TWO,
                DeclareTx::V3(_) => Felt::THREE,
            },
            TxV5::L1Handler(tx) => tx.version,
            TxV5::DeployAccount(tx) => match tx {
                DeployAccountTx::V1(_) => Felt::ONE,
                DeployAccountTx::V3(_) => Felt::THREE,
            },
            TxV5::Deploy(tx) => tx.version,
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

impl From<TxV5> for Tx {
    fn from(tx: TxV5) -> Self {
        match tx {
            TxV5::Invoke(tx) => Tx::Invoke(tx),
            TxV5::Declare(tx) => Tx::Declare(tx),
            TxV5::L1Handler(tx) => Tx::L1Handler(tx),
            TxV5::DeployAccount(tx) => Tx::DeployAccount(tx),
            TxV5::Deploy(tx) => Tx::Deploy(tx),
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

impl From<InvokeTxV3V6> for katana_primitives::transaction::InvokeTxV3 {
    fn from(v6: InvokeTxV3V6) -> Self {
        katana_primitives::transaction::InvokeTxV3 {
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

impl From<DeclareTxV3V6> for katana_primitives::transaction::DeclareTxV3 {
    fn from(v6: DeclareTxV3V6) -> Self {
        katana_primitives::transaction::DeclareTxV3 {
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

impl From<DeployAccountTxV3V6> for katana_primitives::transaction::DeployAccountTxV3 {
    fn from(v6: DeployAccountTxV3V6) -> Self {
        katana_primitives::transaction::DeployAccountTxV3 {
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
    fn from(tx: InvokeTxV6) -> Self {
        match tx {
            InvokeTxV6::V0(tx) => InvokeTx::V0(tx),
            InvokeTxV6::V1(tx) => InvokeTx::V1(tx),
            InvokeTxV6::V3(tx) => InvokeTx::V3(tx.into()),
        }
    }
}

impl From<DeclareTxV6> for DeclareTx {
    fn from(tx: DeclareTxV6) -> Self {
        match tx {
            DeclareTxV6::V0(tx) => DeclareTx::V0(tx),
            DeclareTxV6::V1(tx) => DeclareTx::V1(tx),
            DeclareTxV6::V2(tx) => DeclareTx::V2(tx),
            DeclareTxV6::V3(tx) => DeclareTx::V3(tx.into()),
        }
    }
}

impl From<DeployAccountTxV6> for DeployAccountTx {
    fn from(tx: DeployAccountTxV6) -> Self {
        match tx {
            DeployAccountTxV6::V1(tx) => DeployAccountTx::V1(tx),
            DeployAccountTxV6::V3(tx) => DeployAccountTx::V3(tx.into()),
        }
    }
}

impl From<TxV6> for Tx {
    fn from(tx: TxV6) -> Self {
        match tx {
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
            VersionedTx::V5(tx) => tx.into(),
            VersionedTx::V6(tx) => tx.into(),
            VersionedTx::V7(tx) => tx,
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
            V5(TxV5),
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
