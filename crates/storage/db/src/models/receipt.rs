use katana_primitives::receipt::Receipt;

use crate::models::envelope::{Envelope, EnvelopePayload};

impl EnvelopePayload for Receipt {
    const MAGIC: &[u8; 4] = b"KRCP";
    const NAME: &str = "receipt";
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
    fn receipt_envelope_roundtrip() {
        let receipt = sample_receipt();
        let envelope = ReceiptEnvelope::from(receipt.clone());

        let compressed = envelope.compress().expect("failed to compress receipt envelope");
        assert_eq!(&compressed[..4], b"KRCP");
        assert_eq!(compressed[4], 1); // FORMAT_VERSION

        let decompressed =
            ReceiptEnvelope::decompress(compressed).expect("failed to decompress receipt envelope");

        assert_eq!(decompressed, ReceiptEnvelope::from(receipt));
    }

    #[test]
    fn receipt_envelope_rejects_legacy_postcard_bytes() {
        let receipt = sample_receipt();
        let legacy = postcard::to_stdvec(&receipt).expect("failed to serialize legacy receipt");

        ReceiptEnvelope::decompress(legacy)
            .expect_err("legacy postcard bytes must not be accepted by envelope codec");
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
