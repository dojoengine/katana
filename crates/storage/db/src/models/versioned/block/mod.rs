use katana_primitives::block::Header;

use crate::versioned_type;

mod v6;

// Use the macro to generate the versioned enum and all implementations
versioned_type! {
    VersionedHeader {
        V6 => v6::Header,
        V7 => Header,
    }
}

// Manually implement Default for VersionedHeader since Header has Default
impl Default for VersionedHeader {
    fn default() -> Self {
        Self::V7(Default::default())
    }
}

#[cfg(test)]
mod tests {
    use katana_primitives::block::Header;

    use super::{v6, VersionedHeader};
    use crate::codecs::{Compress, Decompress};

    #[test]
    fn test_versioned_header_v6_to_v7_conversion() {
        let v6_header = v6::Header {
            parent_hash: Default::default(),
            number: 1,
            state_diff_commitment: Default::default(),
            transactions_commitment: Default::default(),
            receipts_commitment: Default::default(),
            events_commitment: Default::default(),
            state_root: Default::default(),
            transaction_count: 0,
            events_count: 0,
            state_diff_length: 0,
            timestamp: 0,
            sequencer_address: Default::default(),
            l1_gas_prices: Default::default(),
            l1_data_gas_prices: Default::default(),
            l1_da_mode: katana_primitives::da::L1DataAvailabilityMode::Blob,
            protocol_version: Default::default(),
        };

        let versioned = VersionedHeader::V6(v6_header.clone());

        // Convert to latest version
        let header: Header = versioned.into();
        assert_eq!(header.number, 1);
        // V6 doesn't have l2_gas_prices, so it should be set to MIN
        assert_eq!(header.l2_gas_prices, katana_primitives::block::GasPrices::MIN);
    }

    #[test]
    fn test_versioned_header_v7_from_conversion() {
        let header = Header { number: 42, ..Default::default() };
        let versioned: VersionedHeader = header.clone().into();

        assert!(matches!(versioned, VersionedHeader::V7(_)));

        // Convert back
        let recovered: Header = versioned.into();
        assert_eq!(header, recovered);
    }

    #[test]
    fn test_versioned_header_compress_decompress() {
        let original = VersionedHeader::V7(Default::default());

        // Compress
        let compressed = original.clone().compress().unwrap();

        // Decompress
        let decompressed = VersionedHeader::decompress(&compressed).unwrap();

        assert_eq!(original, decompressed);
    }

    #[test]
    fn test_backward_compatibility_header_decompression() {
        // Create a V6 header
        let v6_header = v6::Header {
            parent_hash: Default::default(),
            number: 99,
            state_diff_commitment: Default::default(),
            transactions_commitment: Default::default(),
            receipts_commitment: Default::default(),
            events_commitment: Default::default(),
            state_root: Default::default(),
            transaction_count: 0,
            events_count: 0,
            state_diff_length: 0,
            timestamp: 0,
            sequencer_address: Default::default(),
            l1_gas_prices: Default::default(),
            l1_data_gas_prices: Default::default(),
            l1_da_mode: katana_primitives::da::L1DataAvailabilityMode::Blob,
            protocol_version: Default::default(),
        };

        // Serialize it directly (simulating old database data)
        let v6_bytes = postcard::to_stdvec(&v6_header).unwrap();

        // Should be able to decompress as VersionedHeader
        let versioned = VersionedHeader::decompress(&v6_bytes).unwrap();

        match versioned {
            VersionedHeader::V6(h) => assert_eq!(h.number, 99),
            _ => panic!("Expected V6 header"),
        }
    }

    #[test]
    fn test_default_uses_latest_version() {
        let versioned = VersionedHeader::default();
        assert!(matches!(versioned, VersionedHeader::V7(_)));
    }
}
