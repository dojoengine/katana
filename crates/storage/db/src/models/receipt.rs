use katana_primitives::receipt::Receipt;

use crate::models::envelope::{Envelope, EnvelopeError, EnvelopePayload};

impl EnvelopePayload for Receipt {
    const MAGIC: &[u8; 4] = b"KRCP";
    const NAME: &str = "receipt";

    fn from_legacy_bytes(bytes: &[u8]) -> Result<Self, EnvelopeError> {
        postcard::from_bytes(bytes)
            .map_err(|e| EnvelopeError::LegacyDecode { name: Self::NAME, reason: e.to_string() })
    }
}

/// On-disk representation for `Receipts` table values.
pub type ReceiptEnvelope = Envelope<Receipt>;

impl From<ReceiptEnvelope> for Receipt {
    fn from(value: ReceiptEnvelope) -> Self {
        value.inner
    }
}

#[cfg(test)]
mod tests {
    use katana_primitives::receipt::{InvokeTxReceipt, Receipt};

    use super::ReceiptEnvelope;
    use crate::abstraction::{Database, DbTx, DbTxMut};
    use crate::codecs::{Compress, Decompress};
    use crate::models::envelope::EnvelopeError;
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
        assert_eq!(&compressed[..4], b"KRCP");
        assert_eq!(compressed[4], 1); // FORMAT_VERSION
        assert_eq!(compressed[5], 1); // ENCODING_ZSTD

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
        let mut encoded = b"KRCP".to_vec();
        encoded.push(2); // bad version
        encoded.push(1);

        let error = ReceiptEnvelope::do_decompress(&encoded).expect_err("must reject version");
        assert!(matches!(error, EnvelopeError::UnsupportedVersion { version: 2, .. }));
    }

    #[test]
    fn receipt_envelope_rejects_unknown_encoding() {
        let mut encoded = b"KRCP".to_vec();
        encoded.push(1);
        encoded.push(2); // bad encoding

        let error = ReceiptEnvelope::do_decompress(&encoded).expect_err("must reject encoding");
        assert!(matches!(error, EnvelopeError::UnsupportedEncoding { encoding: 2, .. }));
    }

    #[test]
    fn receipt_envelope_rejects_corrupt_zstd_payload() {
        let mut encoded = b"KRCP".to_vec();
        encoded.push(1);
        encoded.push(1);
        encoded.extend_from_slice(&[1, 2, 3, 4]);

        let error =
            ReceiptEnvelope::do_decompress(&encoded).expect_err("must reject corrupt payload");
        assert!(matches!(error, EnvelopeError::ZstdDecompress { .. }));
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
