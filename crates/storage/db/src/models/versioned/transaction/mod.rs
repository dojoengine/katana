use katana_primitives::transaction::Tx;
use serde::{Deserialize, Serialize};

use crate::codecs::{Compress, Decompress};
use crate::error::CodecError;

mod v6;

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

        #[derive(Debug, Deserialize)]
        #[serde(untagged)]
        enum UntaggedVersionedTx {
            V6(v6::Tx),
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

impl From<VersionedTx> for Tx {
    fn from(versioned: VersionedTx) -> Self {
        match versioned {
            VersionedTx::V6(tx) => tx.into(),
            VersionedTx::V7(tx) => tx,
        }
    }
}
