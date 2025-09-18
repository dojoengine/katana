use katana_primitives::transaction::Tx;
use serde::{Deserialize, Serialize};

use crate::codecs::{Compress, Decompress};
use crate::error::CodecError;

mod v6;
mod v7;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(test, derive(::arbitrary::Arbitrary))]
pub enum VersionedTx {
    V6(v6::Tx),
    V7(Tx),
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

        // Try deserializing as V7 first, then fall back to V6
        if let Ok(transaction) = postcard::from_bytes::<Tx>(bytes) {
            return Ok(Self::V7(transaction));
        }

        if let Ok(transaction) = postcard::from_bytes::<v6::Tx>(bytes) {
            return Ok(Self::V6(transaction));
        }

        Err(CodecError::Decompress(
            "failed to deserialize versioned transaction: unknown format".to_string(),
        ))
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
