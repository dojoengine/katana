use katana_primitives::receipt::Receipt;

use crate::codecs::{Compress, Decompress};
use crate::error::CodecError;

/// On-disk representation for `Receipts` table values.
///
/// This wrapper keeps table-specific encoding concerns (envelope header, compression
/// algorithm, and backward-compatibility fallbacks) out of `Receipt` itself.
///
/// New rows are written as `envelope + zstd(postcard(receipt))`.
/// Rows without the envelope are treated as legacy postcard-only bytes.
///
/// ```text
/// +---------+-----------+------------+--------------------------+
/// | magic   | version   | encoding   | payload                  |
/// | 4 bytes | 1 byte    | 1 byte     | variable length          |
/// +---------+-----------+------------+--------------------------+
/// | "KRCP"  | 0x01      | 0x01       | zstd(postcard(receipt))  |
/// +---------+-----------+------------+--------------------------+
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiptEnvelope {
    pub receipt: Receipt,
}

// 4-byte ASCII magic used to identify a Katana receipt envelope.
// Convention: `K` + 3-byte payload identifier, so future table envelopes can follow the same
// recognizable, debuggable namespace pattern.
const RECEIPT_MAGIC: &[u8; 4] = b"KRCP";
const RECEIPT_FORMAT_VERSION: u8 = 1;
const RECEIPT_ENCODING_ZSTD: u8 = 1;
// Encoding `2` is reserved for zstd + dictionary support.
const RECEIPT_HEADER_LEN: usize = RECEIPT_MAGIC.len() + 2;

impl From<Receipt> for ReceiptEnvelope {
    fn from(receipt: Receipt) -> Self {
        Self { receipt }
    }
}

impl From<ReceiptEnvelope> for Receipt {
    fn from(value: ReceiptEnvelope) -> Self {
        value.receipt
    }
}

impl Compress for ReceiptEnvelope {
    type Compressed = Vec<u8>;

    fn compress(self) -> Result<Self::Compressed, CodecError> {
        let serialized =
            postcard::to_stdvec(&self.receipt).map_err(|e| CodecError::Compress(e.to_string()))?;
        let compressed = zstd::encode_all(serialized.as_slice(), 0)
            .map_err(|e| CodecError::Compress(e.to_string()))?;

        // The envelope allows multiple receipt encodings to coexist over time.
        let mut encoded = Vec::with_capacity(RECEIPT_HEADER_LEN + compressed.len());
        encoded.extend_from_slice(RECEIPT_MAGIC);
        encoded.push(RECEIPT_FORMAT_VERSION);
        encoded.push(RECEIPT_ENCODING_ZSTD);
        encoded.extend_from_slice(&compressed);

        Ok(encoded)
    }
}

impl Decompress for ReceiptEnvelope {
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
            let receipt = postcard::from_bytes(&serialized)
                .map_err(|e| CodecError::Decompress(e.to_string()))?;
            Ok(Self { receipt })
        } else {
            // Databases created before receipt compression stored raw postcard bytes.
            postcard::from_bytes::<Receipt>(bytes)
                .map(|receipt| Self { receipt })
                .map_err(|e| CodecError::Decompress(e.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use katana_primitives::receipt::{InvokeTxReceipt, Receipt};

    use super::{
        Compress, Decompress, ReceiptEnvelope, RECEIPT_ENCODING_ZSTD, RECEIPT_FORMAT_VERSION,
        RECEIPT_MAGIC,
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
    fn receipt_envelope_roundtrip_uses_envelope_and_zstd() {
        let receipt = sample_receipt();
        let envelope = ReceiptEnvelope::from(receipt.clone());

        let compressed = envelope.compress().expect("failed to compress receipt envelope");
        assert_eq!(&compressed[..RECEIPT_MAGIC.len()], RECEIPT_MAGIC);
        assert_eq!(compressed[RECEIPT_MAGIC.len()], RECEIPT_FORMAT_VERSION);
        assert_eq!(compressed[RECEIPT_MAGIC.len() + 1], RECEIPT_ENCODING_ZSTD);

        let decompressed =
            ReceiptEnvelope::decompress(compressed).expect("failed to decompress receipt envelope");

        assert_eq!(decompressed, ReceiptEnvelope::from(receipt));
    }

    #[test]
    fn receipt_envelope_decompresses_legacy_postcard_bytes() {
        let receipt = sample_receipt();
        let legacy = postcard::to_stdvec(&receipt).expect("failed to serialize legacy receipt");

        let decompressed =
            ReceiptEnvelope::decompress(legacy).expect("failed to read legacy receipt");

        assert_eq!(decompressed, ReceiptEnvelope::from(receipt));
    }

    #[test]
    fn receipt_envelope_rejects_unknown_version() {
        let mut encoded = RECEIPT_MAGIC.to_vec();
        encoded.push(RECEIPT_FORMAT_VERSION + 1);
        encoded.push(RECEIPT_ENCODING_ZSTD);

        let error = ReceiptEnvelope::decompress(encoded).expect_err("must reject version");
        assert!(matches!(error, CodecError::Decompress(_)));
    }

    #[test]
    fn receipt_envelope_rejects_unknown_encoding() {
        let mut encoded = RECEIPT_MAGIC.to_vec();
        encoded.push(RECEIPT_FORMAT_VERSION);
        encoded.push(RECEIPT_ENCODING_ZSTD + 1);

        let error = ReceiptEnvelope::decompress(encoded).expect_err("must reject encoding");
        assert!(matches!(error, CodecError::Decompress(_)));
    }

    #[test]
    fn receipt_envelope_rejects_corrupt_zstd_payload() {
        let mut encoded = RECEIPT_MAGIC.to_vec();
        encoded.push(RECEIPT_FORMAT_VERSION);
        encoded.push(RECEIPT_ENCODING_ZSTD);
        encoded.extend_from_slice(&[1, 2, 3, 4]);

        let error = ReceiptEnvelope::decompress(encoded).expect_err("must reject corrupt payload");
        assert!(matches!(error, CodecError::Decompress(_)));
    }

    #[test]
    fn receipts_table_roundtrip_uses_receipt_envelope() {
        let db = Db::in_memory().expect("failed to create in-memory db");
        let receipt = sample_receipt();
        let envelope = ReceiptEnvelope::from(receipt.clone());

        let tx = db.tx_mut().expect("failed to open write transaction");
        tx.put::<tables::Receipts>(7, envelope).expect("failed to write receipt");
        tx.commit().expect("failed to commit write transaction");

        let tx = db.tx().expect("failed to open read transaction");
        let stored = tx.get::<tables::Receipts>(7).expect("failed to read receipt");
        tx.commit().expect("failed to commit read transaction");

        assert_eq!(stored.map(Receipt::from), Some(receipt));
    }
}
