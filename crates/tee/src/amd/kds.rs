//! AMD Key Distribution Service (KDS) client
//!
//! Fetch AMD root certificates and compute their hashes for on-chain verification.

use std::collections::HashMap;
use std::path::Path;

use amd_sev_snp_attestation_prover::KDS as SdkKDS;
use amd_sev_snp_attestation_verifier::stub::ProcessorType;
use katana_tracing::trace;
use sha2::{Digest, Sha256};

/// Result of fetching a root certificate
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RootCertInfo {
    /// SHA256 hash of the ARK certificate in DER format (hex with 0x prefix)
    pub ark_hash: String,
    /// Source URL from AMD KDS
    pub source: String,
}

/// Trait for fetching AMD root certificates.
///
/// This trait enables mocking the KDS client in tests.
pub trait RootCertFetcher: Send + Sync {
    /// Fetch root certificate hash for a processor model.
    fn fetch_root_cert_hash(
        &self,
        processor: ProcessorType,
    ) -> Result<RootCertInfo, crate::amd::Error>;
}

/// AMD KDS client for fetching root certificates
pub struct KdsClient {
    inner: SdkKDS,
}

impl std::fmt::Debug for KdsClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `SdkKDS` is an opaque external type that doesn't implement `Debug`.
        f.debug_struct("KdsClient").finish_non_exhaustive()
    }
}

impl KdsClient {
    pub fn new() -> Self {
        Self { inner: SdkKDS::new() }
    }

    /// Fetch root certificate hashes for multiple processor models
    pub fn fetch_root_certs(
        &self,
        processors: &[ProcessorType],
    ) -> Result<HashMap<String, RootCertInfo>, crate::amd::Error> {
        let mut results: HashMap<String, RootCertInfo> = HashMap::with_capacity(processors.len());

        for processor in processors {
            let proc_str = processor
                .to_str()
                .map_err(|e| crate::amd::Error::Prover(format!("Invalid processor type: {e}")))?
                .to_lowercase();

            let info = self.fetch_root_cert_hash(*processor)?;
            results.insert(proc_str, info);
        }

        Ok(results)
    }

    /// Fetch root certificate (ARK) hash for a processor type
    pub fn fetch_root_cert_hash(
        &self,
        processor: ProcessorType,
    ) -> Result<RootCertInfo, crate::amd::Error> {
        let proc_str = processor
            .to_str()
            .map_err(|e| crate::amd::Error::Prover(format!("Invalid processor type: {e}")))?;

        trace!(processor = %proc_str, "Fetching cert chain.");

        let cert_chain = self
            .inner
            .fetch_model_cert_chain(processor)
            .map_err(|e| crate::amd::Error::Prover(format!("Failed to fetch cert chain: {e}")))?;

        // cert_chain is [ASK, ARK] - ARK is index 1
        if cert_chain.len() < 2 {
            return Err(crate::amd::Error::Prover(format!(
                "Expected at least 2 certs in chain, got {}",
                cert_chain.len()
            )));
        }

        let ark_der = &cert_chain[1];
        let ark_hash = Sha256::digest(ark_der.as_ref());

        let ark_hash = format!("0x{}", hex::encode(ark_hash));
        let source = format!("https://kdsintf.amd.com/vcek/v1/{proc_str}/cert_chain");

        Ok(RootCertInfo { ark_hash, source })
    }

    /// Validate fetched root cert against a local .der file
    pub fn validate_against_file(
        &self,
        processor: ProcessorType,
        der_path: &Path,
    ) -> Result<bool, crate::amd::Error> {
        let fetched = self.fetch_root_cert_hash(processor)?;

        let local_der = std::fs::read(der_path).map_err(|e| {
            crate::amd::Error::Prover(format!("Failed to read {}: {e}", der_path.display()))
        })?;

        let local_hash = Sha256::digest(&local_der);
        let local_hash = format!("0x{}", hex::encode(local_hash));

        Ok(fetched.ark_hash == local_hash)
    }
}

impl Default for KdsClient {
    fn default() -> Self {
        Self::new()
    }
}

impl RootCertFetcher for KdsClient {
    fn fetch_root_cert_hash(
        &self,
        processor: ProcessorType,
    ) -> Result<RootCertInfo, crate::amd::Error> {
        // Delegate to the existing method
        KdsClient::fetch_root_cert_hash(self, processor)
    }
}

/// Parse processor type from string
pub fn parse_processor_type(s: &str) -> Option<ProcessorType> {
    match s.trim().to_lowercase().as_str() {
        "milan" => Some(ProcessorType::Milan),
        "genoa" => Some(ProcessorType::Genoa),
        "bergamo" => Some(ProcessorType::Bergamo),
        "siena" => Some(ProcessorType::Siena),
        _ => None,
    }
}
