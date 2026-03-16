use katana_primitives::contract::GenericContractInfo;
use katana_primitives::execution::TypedTransactionExecutionInfo;
use katana_primitives::receipt::Receipt;
use katana_primitives::state::StateUpdates;
use katana_primitives::Felt;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use {postcard, zstd};

use super::{Compress, Decompress};
use crate::error::CodecError;

/// A wrapper type for `Felt` that serializes/deserializes as a 32-byte big-endian array.
///
/// This exists for backward compatibility - older versions of `Felt` used to always serialize as
/// a 32-byte array, but newer versions have changed this behavior. This wrapper ensures
/// consistent serialization format for database storage. However, deserialization is still backward
/// compatible.
///
/// See <https://github.com/starknet-io/types-rs/pull/155> for the breaking change.
///
/// This is temporary and may change in the future.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Felt32(Felt);

impl Serialize for Felt32 {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(&self.0.to_bytes_be())
    }
}

impl<'de> Deserialize<'de> for Felt32 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Felt32(Felt::deserialize(deserializer)?))
    }
}

impl Compress for Felt {
    type Compressed = Vec<u8>;
    fn compress(self) -> Result<Self::Compressed, CodecError> {
        postcard::to_stdvec(&Felt32(self)).map_err(|e| CodecError::Compress(e.to_string()))
    }
}

impl Decompress for Felt {
    fn decompress<B: AsRef<[u8]>>(bytes: B) -> Result<Self, CodecError> {
        let wrapper: Felt32 = postcard::from_bytes(bytes.as_ref())
            .map_err(|e| CodecError::Decompress(e.to_string()))?;
        Ok(wrapper.0)
    }
}

use crate::models::block::StoredBlockBodyIndices;
use crate::models::contract::ContractInfoChangeList;
use crate::models::list::BlockChangeList;
use crate::models::stage::{ExecutionCheckpoint, MigrationCheckpoint, PruningCheckpoint};
use crate::models::state::HistoricalStateRetention;
use crate::models::trie::TrieDatabaseValue;

macro_rules! impl_compress_and_decompress_for_table_values {
    ($($name:ty),*) => {
        $(
            impl Compress for $name {
                type Compressed = Vec<u8>;
                fn compress(self) -> Result<Self::Compressed, crate::error::CodecError> {
                    postcard::to_stdvec(&self)
                        .map_err(|e| CodecError::Compress(e.to_string()))
                }
            }

            impl Decompress for $name {
                fn decompress<B: AsRef<[u8]>>(bytes: B) -> Result<Self, crate::error::CodecError> {
                    postcard::from_bytes(bytes.as_ref()).map_err(|e| CodecError::Decompress(e.to_string()))
                }
            }
        )*
    }
}

impl Compress for TypedTransactionExecutionInfo {
    type Compressed = Vec<u8>;
    fn compress(self) -> Result<Self::Compressed, crate::error::CodecError> {
        let serialized = postcard::to_stdvec(&self).unwrap();
        zstd::encode_all(serialized.as_slice(), 0).map_err(|e| CodecError::Compress(e.to_string()))
    }
}

impl Decompress for TypedTransactionExecutionInfo {
    fn decompress<B: AsRef<[u8]>>(bytes: B) -> Result<Self, crate::error::CodecError> {
        let compressed = bytes.as_ref();
        let serialized =
            zstd::decode_all(compressed).map_err(|e| CodecError::Decompress(e.to_string()))?;
        postcard::from_bytes(&serialized).map_err(|e| CodecError::Decompress(e.to_string()))
    }
}

// `Receipt` intentionally stays postcard-only. Compression envelopes are handled by
// `models::ReceiptEnvelope`, which is used as the `Receipts` table value.
impl_compress_and_decompress_for_table_values!(
    u64,
    Receipt,
    TrieDatabaseValue,
    BlockChangeList,
    ExecutionCheckpoint,
    MigrationCheckpoint,
    PruningCheckpoint,
    HistoricalStateRetention,
    GenericContractInfo,
    StoredBlockBodyIndices,
    ContractInfoChangeList,
    StateUpdates
);

#[cfg(test)]
mod tests {
    use katana_primitives::receipt::{InvokeTxReceipt, Receipt};

    use super::{Compress, Decompress};
    use crate::models::ReceiptEnvelope;

    fn sample_receipt() -> Receipt {
        Receipt::Invoke(InvokeTxReceipt {
            revert_error: Some("boom".into()),
            events: Vec::new(),
            fee: Default::default(),
            messages_sent: Vec::new(),
            execution_resources: Default::default(),
        })
    }

    #[test]
    fn receipt_roundtrip_uses_postcard() {
        let receipt = sample_receipt();

        let compressed = receipt.clone().compress().expect("failed to compress receipt");
        let expected = postcard::to_stdvec(&receipt).expect("failed to serialize postcard receipt");
        assert_eq!(compressed, expected);

        let decompressed =
            <Receipt as Decompress>::decompress(compressed).expect("failed to decompress receipt");

        assert_eq!(decompressed, receipt);
    }

    #[test]
    fn receipt_does_not_decode_envelope_bytes() {
        let receipt = sample_receipt();
        let envelope_bytes =
            ReceiptEnvelope::from(receipt).compress().expect("failed to compress receipt envelope");
        let err = <Receipt as Decompress>::decompress(envelope_bytes)
            .expect_err("receipt codec must remain postcard-only");
        assert!(matches!(err, crate::error::CodecError::Decompress(_)));
    }
}
