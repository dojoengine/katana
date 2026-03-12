use katana_primitives::transaction::Tx;
use serde::{Deserialize, Serialize};

use crate::models::envelope::{EnvelopeError, EnvelopePayload};

mod v6;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "arbitrary"), derive(::arbitrary::Arbitrary))]
pub enum VersionedTx {
    V6(v6::Tx),
    V7(Tx),
}

impl From<Tx> for VersionedTx {
    fn from(tx: Tx) -> Self {
        VersionedTx::V7(tx)
    }
}

impl EnvelopePayload for VersionedTx {
    const MAGIC: &[u8; 4] = b"KTXN";
    const NAME: &str = "transaction";

    fn from_legacy_bytes(bytes: &[u8]) -> Result<Self, EnvelopeError> {
        if let Ok(tx) = postcard::from_bytes::<Self>(bytes) {
            return Ok(tx);
        }

        if let Ok(transaction) = postcard::from_bytes::<Tx>(bytes) {
            return Ok(Self::V7(transaction));
        }

        if let Ok(transaction) = postcard::from_bytes::<v6::Tx>(bytes) {
            return Ok(Self::V6(transaction));
        }

        Err(EnvelopeError::LegacyDecode {
            name: Self::NAME,
            reason: "unknown transaction format".to_string(),
        })
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
