use katana_primitives::contract::GenericContractInfo;
use katana_primitives::execution::TypedTransactionExecutionInfo;
use katana_primitives::receipt::Receipt;
use katana_primitives::Felt;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use {postcard, zstd};

use super::{Compress, Decompress};
use crate::error::CodecError;

// Receipt rows now use an explicit envelope so new writers can store compressed payloads while
// readers remain compatible with legacy rows that were stored as raw postcard bytes.
const RECEIPT_MAGIC: &[u8; 4] = b"KRCP";
const RECEIPT_FORMAT_VERSION: u8 = 1;
// Encoding `2` is reserved for zstd + dictionary support.
const RECEIPT_ENCODING_ZSTD: u8 = 1;
const RECEIPT_HEADER_LEN: usize = RECEIPT_MAGIC.len() + 2;

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
use crate::models::list::BlockList;
use crate::models::stage::{ExecutionCheckpoint, PruningCheckpoint};
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

impl Compress for Receipt {
    type Compressed = Vec<u8>;

    fn compress(self) -> Result<Self::Compressed, CodecError> {
        let serialized =
            postcard::to_stdvec(&self).map_err(|e| CodecError::Compress(e.to_string()))?;
        let compressed = zstd::encode_all(serialized.as_slice(), 0)
            .map_err(|e| CodecError::Compress(e.to_string()))?;

        // Prefix the payload with a small format header so future receipt encodings can coexist
        // with both legacy postcard rows and this zstd-backed format.
        let mut encoded = Vec::with_capacity(RECEIPT_HEADER_LEN + compressed.len());
        encoded.extend_from_slice(RECEIPT_MAGIC);
        encoded.push(RECEIPT_FORMAT_VERSION);
        encoded.push(RECEIPT_ENCODING_ZSTD);
        encoded.extend_from_slice(&compressed);

        Ok(encoded)
    }
}

impl Decompress for Receipt {
    fn decompress<B: AsRef<[u8]>>(bytes: B) -> Result<Self, CodecError> {
        let bytes = bytes.as_ref();

        if bytes.starts_with(RECEIPT_MAGIC) {
            if bytes.len() < RECEIPT_HEADER_LEN {
                return Err(CodecError::Decompress("Incomplete receipt envelope".into()));
            }

            let version = bytes[RECEIPT_MAGIC.len()];
            if version != RECEIPT_FORMAT_VERSION {
                return Err(CodecError::Decompress(format!(
                    "Unsupported receipt encoding version: {version}"
                )));
            }

            let encoding = bytes[RECEIPT_MAGIC.len() + 1];
            if encoding != RECEIPT_ENCODING_ZSTD {
                return Err(CodecError::Decompress(format!(
                    "Unsupported receipt encoding: {encoding}"
                )));
            }

            let serialized = zstd::decode_all(&bytes[RECEIPT_HEADER_LEN..])
                .map_err(|e| CodecError::Decompress(e.to_string()))?;

            postcard::from_bytes(&serialized).map_err(|e| CodecError::Decompress(e.to_string()))
        } else {
            // Databases created before receipt compression stored raw postcard bytes.
            // Only rows without the envelope use this fallback; malformed enveloped rows are
            // treated as corruption instead of being reinterpreted as legacy data.
            postcard::from_bytes(bytes).map_err(|e| CodecError::Decompress(e.to_string()))
        }
    }
}

impl_compress_and_decompress_for_table_values!(
    u64,
    TrieDatabaseValue,
    BlockList,
    ExecutionCheckpoint,
    PruningCheckpoint,
    GenericContractInfo,
    StoredBlockBodyIndices,
    ContractInfoChangeList
);

#[cfg(test)]
mod tests {
    use katana_primitives::receipt::{InvokeTxReceipt, Receipt};

    use super::{
        Compress, Decompress, RECEIPT_ENCODING_ZSTD, RECEIPT_FORMAT_VERSION, RECEIPT_MAGIC,
    };
    use crate::abstraction::{Database, DbTx, DbTxMut};
    use crate::error::CodecError;
    use crate::{tables, Db};

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
    fn receipt_roundtrip_uses_envelope_and_zstd() {
        let receipt = sample_receipt();

        let compressed = receipt.clone().compress().expect("failed to compress receipt");
        assert_eq!(&compressed[..RECEIPT_MAGIC.len()], RECEIPT_MAGIC);
        assert_eq!(compressed[RECEIPT_MAGIC.len()], RECEIPT_FORMAT_VERSION);
        assert_eq!(compressed[RECEIPT_MAGIC.len() + 1], RECEIPT_ENCODING_ZSTD);

        let decompressed =
            <Receipt as Decompress>::decompress(compressed).expect("failed to decompress receipt");

        assert_eq!(decompressed, receipt);
    }

    #[test]
    fn receipt_decompresses_legacy_postcard_bytes() {
        let receipt = sample_receipt();
        let legacy = postcard::to_stdvec(&receipt).expect("failed to serialize legacy receipt");

        let decompressed =
            <Receipt as Decompress>::decompress(legacy).expect("failed to read legacy receipt");

        assert_eq!(decompressed, receipt);
    }

    #[test]
    fn receipt_rejects_unknown_envelope_version() {
        let mut encoded = RECEIPT_MAGIC.to_vec();
        encoded.push(RECEIPT_FORMAT_VERSION + 1);
        encoded.push(RECEIPT_ENCODING_ZSTD);

        let error = <Receipt as Decompress>::decompress(encoded).expect_err("must reject version");
        assert!(matches!(error, CodecError::Decompress(_)));
    }

    #[test]
    fn receipt_rejects_unknown_envelope_encoding() {
        let mut encoded = RECEIPT_MAGIC.to_vec();
        encoded.push(RECEIPT_FORMAT_VERSION);
        encoded.push(RECEIPT_ENCODING_ZSTD + 1);

        let error = <Receipt as Decompress>::decompress(encoded).expect_err("must reject encoding");
        assert!(matches!(error, CodecError::Decompress(_)));
    }

    #[test]
    fn receipt_rejects_corrupt_zstd_payload() {
        let mut encoded = RECEIPT_MAGIC.to_vec();
        encoded.push(RECEIPT_FORMAT_VERSION);
        encoded.push(RECEIPT_ENCODING_ZSTD);
        encoded.extend_from_slice(&[1, 2, 3, 4]);

        let error =
            <Receipt as Decompress>::decompress(encoded).expect_err("must reject corrupt payload");
        assert!(matches!(error, CodecError::Decompress(_)));
    }

    #[test]
    fn receipts_table_roundtrip_uses_custom_codec() {
        let db = Db::in_memory().expect("failed to create in-memory db");
        let receipt = sample_receipt();

        let tx = db.tx_mut().expect("failed to open write transaction");
        tx.put::<tables::Receipts>(7, receipt.clone()).expect("failed to write receipt");
        tx.commit().expect("failed to commit write transaction");

        let tx = db.tx().expect("failed to open read transaction");
        let stored = tx.get::<tables::Receipts>(7).expect("failed to read receipt");
        tx.commit().expect("failed to commit read transaction");

        assert_eq!(stored, Some(receipt));
    }
}
