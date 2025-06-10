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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TxV6 {
    Invoke(InvokeTx),
    Declare(DeclareTx),
    L1Handler(L1HandlerTx),
    DeployAccount(DeployAccountTx),
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
                InvokeTx::V0(_) => Felt::ZERO,
                InvokeTx::V1(_) => Felt::ONE,
                InvokeTx::V3(_) => Felt::THREE,
            },
            TxV6::Declare(tx) => match tx {
                DeclareTx::V0(_) => Felt::ZERO,
                DeclareTx::V1(_) => Felt::ONE,
                DeclareTx::V2(_) => Felt::TWO,
                DeclareTx::V3(_) => Felt::THREE,
            },
            TxV6::L1Handler(tx) => tx.version,
            TxV6::DeployAccount(tx) => match tx {
                DeployAccountTx::V1(_) => Felt::ONE,
                DeployAccountTx::V3(_) => Felt::THREE,
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

impl From<TxV6> for Tx {
    fn from(tx: TxV6) -> Self {
        match tx {
            TxV6::Invoke(tx) => Tx::Invoke(tx),
            TxV6::Declare(tx) => Tx::Declare(tx),
            TxV6::L1Handler(tx) => Tx::L1Handler(tx),
            TxV6::DeployAccount(tx) => Tx::DeployAccount(tx),
            TxV6::Deploy(tx) => Tx::Deploy(tx),
        }
    }
}

impl From<VersionedTx> for Tx {
    fn from(versioned: VersionedTx) -> Self {
        match versioned {
            VersionedTx::V7(tx) => tx,
            VersionedTx::V6(tx) => tx.into(),
            VersionedTx::V5(tx) => tx.into(),
        }
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
        if let Ok(tx) = postcard::from_bytes::<Tx>(bytes.as_ref()) {
            return Ok(VersionedTx::V7(tx));
        }

        if let Ok(tx) = postcard::from_bytes::<TxV6>(bytes.as_ref()) {
            return Ok(VersionedTx::V6(tx));
        }

        if let Ok(tx) = postcard::from_bytes::<TxV5>(bytes.as_ref()) {
            return Ok(VersionedTx::V5(tx));
        }

        postcard::from_bytes::<VersionedTx>(bytes.as_ref())
            .map_err(|e| CodecError::Decompress(e.to_string()))
    }
}
