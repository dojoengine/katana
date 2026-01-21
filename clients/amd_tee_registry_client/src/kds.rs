//! AMD Key Distribution Service (KDS) client
//!
//! Fetch AMD root certificates and compute their hashes for on-chain verification.

use amd_sev_snp_attestation_prover::KDS as SdkKDS;
use amd_sev_snp_attestation_verifier::stub::ProcessorType;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;

/// Result of fetching a root certificate
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RootCertInfo {
    /// SHA256 hash of the ARK certificate in DER format (hex with 0x prefix)
    pub ark_hash: String,
    /// Source URL from AMD KDS
    pub source: String,
}

/// AMD KDS client for fetching root certificates
pub struct KdsClient {
    inner: SdkKDS,
}

impl KdsClient {
    pub fn new() -> Self {
        Self {
            inner: SdkKDS::new(),
        }
    }

    /// Fetch root certificate (ARK) hash for a processor type
    pub fn fetch_root_cert_hash(&self, processor: ProcessorType) -> Result<RootCertInfo, crate::Error> {
        let proc_str = processor.to_str()
            .map_err(|e| crate::Error::Prover(format!("Invalid processor type: {}", e)))?;

        tracing::info!("Fetching cert chain for {}...", proc_str);

        let cert_chain = self.inner.fetch_model_cert_chain(processor)
            .map_err(|e| crate::Error::Prover(format!("Failed to fetch cert chain: {}", e)))?;

        // cert_chain is [ASK, ARK] - ARK is index 1
        if cert_chain.len() < 2 {
            return Err(crate::Error::Prover(format!(
                "Expected at least 2 certs in chain, got {}",
                cert_chain.len()
            )));
        }

        let ark_der = &cert_chain[1];
        let ark_hash = Sha256::digest(ark_der.as_ref());
        let ark_hash_hex = format!("0x{}", hex::encode(ark_hash));

        Ok(RootCertInfo {
            ark_hash: ark_hash_hex,
            source: format!("https://kdsintf.amd.com/vcek/v1/{}/cert_chain", proc_str),
        })
    }

    /// Fetch root certificate hashes for multiple processor types
    pub fn fetch_root_certs(&self, processors: &[ProcessorType]) -> Result<HashMap<String, RootCertInfo>, crate::Error> {
        let mut results = HashMap::new();

        for processor in processors {
            let proc_str = processor.to_str()
                .map_err(|e| crate::Error::Prover(format!("Invalid processor type: {}", e)))?
                .to_lowercase();

            let info = self.fetch_root_cert_hash(*processor)?;
            results.insert(proc_str, info);
        }

        Ok(results)
    }

    /// Validate fetched root cert against a local .der file
    pub fn validate_against_file(&self, processor: ProcessorType, der_path: &Path) -> Result<bool, crate::Error> {
        let fetched = self.fetch_root_cert_hash(processor)?;

        let local_der = std::fs::read(der_path)
            .map_err(|e| crate::Error::Prover(format!("Failed to read {}: {}", der_path.display(), e)))?;

        let local_hash = Sha256::digest(&local_der);
        let local_hash_hex = format!("0x{}", hex::encode(local_hash));

        Ok(fetched.ark_hash == local_hash_hex)
    }
}

impl Default for KdsClient {
    fn default() -> Self {
        Self::new()
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
