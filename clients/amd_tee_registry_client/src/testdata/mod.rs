//! Test data for AMD SEV-SNP certificates
//!
//! This module provides embedded certificates from AMD KDS for testing purposes.
//! The certificates are fetched directly from AMD's Key Distribution Service.
//!
//! ## Sources
//! - AMD KDS: `https://kdsintf.amd.com`
//! - `virtee/sev` crate: https://github.com/virtee/sev
//! - `google/go-sev-guest`: https://github.com/google/go-sev-guest

use crate::types::ProcessorType;

// ============================================================================
// Embedded PEM Certificate Chains (fetched from AMD KDS)
// Chain order: ASK (first), ARK (second)
// ============================================================================

/// Milan certificate chain in PEM format (ASK + ARK)
pub const MILAN_CHAIN_PEM: &str = include_str!("milan_chain.pem");

/// Genoa certificate chain in PEM format (ASK + ARK)
pub const GENOA_CHAIN_PEM: &str = include_str!("genoa_chain.pem");

// ============================================================================
// Helper Functions
// ============================================================================

/// Get the certificate chain PEM for a processor type
pub fn get_chain_pem(processor: ProcessorType) -> Option<&'static str> {
    match processor {
        ProcessorType::Milan => Some(MILAN_CHAIN_PEM),
        ProcessorType::Genoa => Some(GENOA_CHAIN_PEM),
        // Bergamo and Siena not yet available on KDS
        ProcessorType::Bergamo | ProcessorType::Siena => None,
    }
}

/// Parse a certificate chain PEM into (ASK DER, ARK DER)
/// Returns (ask, ark) in DER format
pub fn parse_chain_pem(pem_data: &str) -> Result<(Vec<u8>, Vec<u8>), pem::PemError> {
    let pems: Vec<pem::Pem> = pem::parse_many(pem_data)?;
    if pems.len() < 2 {
        return Err(pem::PemError::MalformedFraming);
    }
    // KDS order: ASK first, ARK second
    let ask = pems[0].contents().to_vec();
    let ark = pems[1].contents().to_vec();
    Ok((ask, ark))
}

/// Get the ARK certificate in DER format for a processor type
pub fn get_ark_der(processor: ProcessorType) -> Option<Vec<u8>> {
    get_chain_pem(processor).and_then(|pem| parse_chain_pem(pem).ok().map(|(_, ark)| ark))
}

/// Get the ASK certificate in DER format for a processor type
pub fn get_ask_der(processor: ProcessorType) -> Option<Vec<u8>> {
    get_chain_pem(processor).and_then(|pem| parse_chain_pem(pem).ok().map(|(ask, _)| ask))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_milan_chain_pem_valid() {
        assert!(!MILAN_CHAIN_PEM.is_empty());
        // Should contain two certificates
        let count = MILAN_CHAIN_PEM.matches("BEGIN CERTIFICATE").count();
        assert_eq!(count, 2, "Milan chain should have 2 certificates");
    }

    #[test]
    fn test_genoa_chain_pem_valid() {
        assert!(!GENOA_CHAIN_PEM.is_empty());
        let count = GENOA_CHAIN_PEM.matches("BEGIN CERTIFICATE").count();
        assert_eq!(count, 2, "Genoa chain should have 2 certificates");
    }

    #[test]
    fn test_parse_chain_pem() {
        let (ask, ark) = parse_chain_pem(MILAN_CHAIN_PEM).expect("Failed to parse Milan chain");

        // Both should be non-empty DER certificates
        assert!(!ask.is_empty());
        assert!(!ark.is_empty());

        // DER certificates start with 0x30 (SEQUENCE tag)
        assert_eq!(ask[0], 0x30, "ASK should start with SEQUENCE tag");
        assert_eq!(ark[0], 0x30, "ARK should start with SEQUENCE tag");

        println!("Milan: ASK={} bytes, ARK={} bytes", ask.len(), ark.len());
    }

    #[test]
    fn test_get_ark_der() {
        let milan_ark = get_ark_der(ProcessorType::Milan).expect("Milan ARK should be available");
        let genoa_ark = get_ark_der(ProcessorType::Genoa).expect("Genoa ARK should be available");

        assert!(!milan_ark.is_empty());
        assert!(!genoa_ark.is_empty());

        // Bergamo/Siena not available
        assert!(get_ark_der(ProcessorType::Bergamo).is_none());
        assert!(get_ark_der(ProcessorType::Siena).is_none());
    }

    // ========================================================================
    // Integration tests - compare embedded certs with live KDS
    // ========================================================================

    #[tokio::test]
    async fn test_embedded_milan_matches_kds() {
        use crate::KdsClient;

        let client = KdsClient::new();
        let chain = client
            .fetch_cert_chain(ProcessorType::Milan)
            .await
            .expect("Failed to fetch Milan cert chain from KDS");

        let embedded_ark = get_ark_der(ProcessorType::Milan).expect("Embedded Milan ARK");
        let embedded_ask = get_ask_der(ProcessorType::Milan).expect("Embedded Milan ASK");

        assert_eq!(
            chain.ark, embedded_ark,
            "Milan ARK from KDS doesn't match embedded certificate"
        );
        assert_eq!(
            chain.ask, embedded_ask,
            "Milan ASK from KDS doesn't match embedded certificate"
        );

        println!("✓ Milan embedded certs match KDS");
    }

    #[tokio::test]
    async fn test_embedded_genoa_matches_kds() {
        use crate::KdsClient;

        let client = KdsClient::new();
        let chain = client
            .fetch_cert_chain(ProcessorType::Genoa)
            .await
            .expect("Failed to fetch Genoa cert chain from KDS");

        let embedded_ark = get_ark_der(ProcessorType::Genoa).expect("Embedded Genoa ARK");
        let embedded_ask = get_ask_der(ProcessorType::Genoa).expect("Embedded Genoa ASK");

        assert_eq!(
            chain.ark, embedded_ark,
            "Genoa ARK from KDS doesn't match embedded certificate"
        );
        assert_eq!(
            chain.ask, embedded_ask,
            "Genoa ASK from KDS doesn't match embedded certificate"
        );

        println!("✓ Genoa embedded certs match KDS");
    }
}
