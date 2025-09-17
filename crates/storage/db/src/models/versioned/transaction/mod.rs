use katana_primitives::transaction::Tx;

mod v6;

versioned_type! {
    VersionedTx {
        V6 => v6::Tx,
        V7 => Tx,
    }
}

#[cfg(test)]
mod tests {
    use katana_primitives::transaction::{InvokeTx, Tx};

    use super::{v6, VersionedTx};
    use crate::codecs::{Compress, Decompress};

    #[test]
    fn test_versioned_tx_v6_to_v7_conversion() {
        let v6_tx = v6::Tx::Invoke(v6::InvokeTx::V0(Default::default()));
        let versioned = VersionedTx::V6(v6_tx);

        // Convert to latest version
        let tx: Tx = versioned.into();
        assert!(matches!(tx, Tx::Invoke(InvokeTx::V0(_))));
    }

    #[test]
    fn test_versioned_tx_v7_from_conversion() {
        let tx = Tx::Invoke(InvokeTx::V1(Default::default()));
        let versioned: VersionedTx = tx.clone().into();

        assert!(matches!(versioned, VersionedTx::V7(_)));

        // Convert back
        let recovered: Tx = versioned.into();
        assert_eq!(tx, recovered);
    }

    #[test]
    fn test_versioned_tx_compress_decompress() {
        let original = VersionedTx::V7(Tx::Invoke(InvokeTx::V1(Default::default())));

        // Compress
        let compressed = original.clone().compress().unwrap();

        // Decompress
        let decompressed = VersionedTx::decompress(&compressed).unwrap();

        assert_eq!(original, decompressed);
    }

    #[test]
    fn test_backward_compatibility_decompression() {
        // Test with a simple V6 transaction type
        // The key thing we're testing is that the decompress fallback chain works
        // In reality, V6 and V7 might be similar enough that V7 deserialization works for some V6
        // data This is OK - what matters is that old data can be read, not which variant is
        // chosen
        let v6_tx = v6::Tx::Invoke(v6::InvokeTx::V0(Default::default()));

        // Create a properly versioned V6 enum value and serialize it
        let versioned_v6 = VersionedTx::V6(v6_tx.clone());
        let v6_bytes = postcard::to_stdvec(&versioned_v6).unwrap();

        // Should be able to decompress the versioned enum
        let versioned = VersionedTx::decompress(&v6_bytes).unwrap();

        // It should deserialize as V6 since we explicitly serialized a V6 variant
        assert!(matches!(versioned, VersionedTx::V6(_)));

        // Also test that we can convert it to the latest type
        let latest: Tx = versioned.into();
        assert!(matches!(latest, Tx::Invoke(_)));
    }

    #[test]
    fn test_versioned_enum_decompression() {
        // Test that we can decompress a versioned enum that was previously serialized
        let original = VersionedTx::V6(v6::Tx::Invoke(v6::InvokeTx::V0(Default::default())));
        let bytes = postcard::to_stdvec(&original).unwrap();

        let decompressed = VersionedTx::decompress(&bytes).unwrap();
        assert_eq!(original, decompressed);
    }
}
