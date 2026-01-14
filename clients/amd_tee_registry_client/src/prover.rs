//! AMD SEV-SNP Attestation SP1 Prover
//!
//! This module provides functionality to generate SP1 Groth16 proofs
//! from AMD SEV-SNP attestation reports.
//!
//! # Overview
//!
//! The prover takes a raw AMD SEV-SNP attestation report (1184 bytes)
//! and generates a zero-knowledge proof that can be verified on-chain.
//!
//! # Proof Types
//!
//! - **Groth16**: Compact proofs (~260 bytes) for on-chain verification
//! - **Compressed**: Larger proofs for off-chain use
//!
//! # Usage
//!
//! ```no_run
//! use amd_tee_registry_client::{AmdAttestationProver, ProverConfig};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create prover with default config (reads from environment)
//! let prover = AmdAttestationProver::new(ProverConfig::from_env());
//!
//! // Raw attestation report bytes (1184 bytes)
//! let report_bytes: Vec<u8> = vec![/* ... */];
//!
//! // Generate Groth16 proof
//! let proof = prover.prove(&report_bytes).await?;
//!
//! println!("Proof generated: {} bytes", proof.onchain_proof.len());
//! # Ok(())
//! # }
//! ```
//!
//! # Environment Variables
//!
//! - `SP1_PROVER`: Prover mode - "mock", "cpu", or "network"
//! - `NETWORK_PRIVATE_KEY`: Private key for SP1 Prover Network
//! - `SKIP_TIME_VALIDITY_CHECK`: Skip certificate time validation
//!
//! # Note on "insecure random" Warning
//!
//! You may see a warning about "insecure random number generator" during
//! proof generation. This is expected and does NOT affect security:
//!
//! - The warning occurs during local execution (cycle counting)
//! - For network proving, the actual proof is generated on secure GPU clusters
//! - The final Groth16 proof is cryptographically secure

use crate::Error;
use alloy_primitives::Bytes;
use amd_sev_snp_attestation_prover::{AmdSevSnpProver, ProverConfig as SdkProverConfig};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, info};

// Re-export OnchainProof for convenience
pub use amd_sev_snp_attestation_prover::OnchainProof;

/// Expected size of AMD SEV-SNP attestation report in bytes
pub const ATTESTATION_REPORT_SIZE: usize = 1184;

/// Configuration for the AMD attestation prover.
#[derive(Debug, Clone, Default)]
pub struct ProverConfig {
    /// Private key for SP1 network proving (optional for mock mode)
    pub private_key: Option<String>,
    /// SP1 RPC URL (optional, uses default if not specified)
    pub rpc_url: Option<String>,
    /// Skip time validity check (useful for testing with old attestations)
    pub skip_time_validity_check: bool,
}

impl ProverConfig {
    /// Create a new prover config with explicit values.
    pub fn new(
        private_key: Option<String>,
        rpc_url: Option<String>,
        skip_time_validity_check: bool,
    ) -> Self {
        Self {
            private_key,
            rpc_url,
            skip_time_validity_check,
        }
    }

    /// Create config from environment variables.
    ///
    /// Reads:
    /// - `NETWORK_PRIVATE_KEY` - Private key for SP1 network proving (preferred)
    /// - `SP1_PRIVATE_KEY` - Private key for network proving (fallback)
    /// - `SP1_RPC_URL` - RPC URL for SP1 network
    /// - `SKIP_TIME_VALIDITY_CHECK` - Skip time validity (true/false)
    pub fn from_env() -> Self {
        Self {
            // NETWORK_PRIVATE_KEY is the standard env var used by SP1 SDK
            private_key: std::env::var("NETWORK_PRIVATE_KEY")
                .ok()
                .or_else(|| std::env::var("SP1_PRIVATE_KEY").ok()),
            rpc_url: std::env::var("SP1_RPC_URL").ok(),
            skip_time_validity_check: std::env::var("SKIP_TIME_VALIDITY_CHECK")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(false),
        }
    }

    /// Check if network proving is configured.
    pub fn has_network_key(&self) -> bool {
        self.private_key.is_some()
    }
}

/// AMD SEV-SNP Attestation Prover
///
/// Generates SP1 Groth16 proofs from AMD SEV-SNP attestation reports.
/// These proofs can be verified on-chain to prove TEE attestation.
#[derive(Debug, Clone)]
pub struct AmdAttestationProver {
    config: ProverConfig,
}

impl AmdAttestationProver {
    /// Create a new prover with the given configuration.
    pub fn new(config: ProverConfig) -> Self {
        Self { config }
    }

    /// Create a prover with configuration from environment variables.
    pub fn from_env() -> Self {
        Self::new(ProverConfig::from_env())
    }

    /// Get the prover configuration.
    pub fn config(&self) -> &ProverConfig {
        &self.config
    }

    /// Generate an SP1 Groth16 proof from a raw attestation report.
    ///
    /// # Arguments
    /// * `report_bytes` - Raw AMD SEV-SNP attestation report (1184 bytes)
    ///
    /// # Returns
    /// * `OnchainProof` - The generated proof with metadata
    ///
    /// # Errors
    /// * Returns error if report size is invalid
    /// * Returns error if proof generation fails
    pub async fn prove(&self, report_bytes: &[u8]) -> Result<OnchainProof, Error> {
        self.prove_with_timestamp(report_bytes, current_timestamp()?)
            .await
    }

    /// Generate an SP1 Groth16 proof with a specific timestamp.
    ///
    /// This is useful for testing with specific timestamps or for
    /// reproducibility.
    ///
    /// # Arguments
    /// * `report_bytes` - Raw AMD SEV-SNP attestation report (1184 bytes)
    /// * `timestamp` - Unix timestamp for certificate validation
    pub async fn prove_with_timestamp(
        &self,
        report_bytes: &[u8],
        timestamp: u64,
    ) -> Result<OnchainProof, Error> {
        // Validate report size
        if report_bytes.len() != ATTESTATION_REPORT_SIZE {
            return Err(Error::InvalidReportSize {
                expected: ATTESTATION_REPORT_SIZE,
                actual: report_bytes.len(),
            });
        }

        info!("Starting SP1 proof generation for attestation report");
        debug!(
            "Report size: {} bytes, timestamp: {}",
            report_bytes.len(),
            timestamp
        );

        // Apply configuration
        self.apply_config();

        // Clone data for the blocking task
        let report_bytes = report_bytes.to_vec();

        // Run the blocking prover in a separate thread
        // The AMD SDK prover uses its own internal tokio runtime via block_on
        let proof = tokio::task::spawn_blocking(move || {
            // Create prover with SP1 configuration
            let prover_config = SdkProverConfig::sp1();
            let prover = AmdSevSnpProver::new(prover_config, None);

            info!("SP1 prover initialized, generating proof...");

            // Generate the proof
            // vek_certs=None means the prover will fetch them from AMD KDS
            prover.prove_attestation_report(timestamp, Bytes::from(report_bytes), None)
        })
        .await
        .map_err(|e| Error::Prover(format!("Task join error: {}", e)))?
        .map_err(|e| Error::Prover(format!("Proof generation failed: {}", e)))?;

        info!(
            "Proof generated successfully. Verifier ID: {}",
            proof.program_id.verifier_id
        );

        Ok(proof)
    }

    /// Verify that a proof has valid structure.
    ///
    /// This performs local validation without on-chain verification.
    pub fn verify_proof_structure(proof: &OnchainProof) -> Result<(), Error> {
        if proof.onchain_proof.is_empty() {
            return Err(Error::Prover("Proof bytes are empty".to_string()));
        }

        if proof.program_id.verifier_id.is_zero() {
            return Err(Error::Prover("Verifier ID is zero".to_string()));
        }

        info!(
            "Proof structure valid. Type: {:?}, Size: {} bytes",
            proof.zktype,
            proof.onchain_proof.len()
        );

        Ok(())
    }

    /// Apply configuration to environment variables.
    fn apply_config(&self) {
        if let Some(ref key) = self.config.private_key {
            std::env::set_var("NETWORK_PRIVATE_KEY", key);
        }
        if let Some(ref url) = self.config.rpc_url {
            std::env::set_var("SP1_RPC_URL", url);
        }
        if self.config.skip_time_validity_check {
            std::env::set_var("SKIP_TIME_VALIDITY_CHECK", "true");
        }
    }
}

/// Get the current Unix timestamp.
fn current_timestamp() -> Result<u64, Error> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|e| Error::Prover(format!("Failed to get timestamp: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_from_env() {
        // Set test environment
        std::env::set_var("NETWORK_PRIVATE_KEY", "test_key");
        std::env::set_var("SKIP_TIME_VALIDITY_CHECK", "true");

        let config = ProverConfig::from_env();
        assert_eq!(config.private_key, Some("test_key".to_string()));
        assert!(config.skip_time_validity_check);
        assert!(config.has_network_key());

        // Clean up
        std::env::remove_var("NETWORK_PRIVATE_KEY");
        std::env::remove_var("SKIP_TIME_VALIDITY_CHECK");
    }

    #[test]
    fn test_config_fallback_to_sp1_private_key() {
        std::env::remove_var("NETWORK_PRIVATE_KEY");
        std::env::set_var("SP1_PRIVATE_KEY", "fallback_key");

        let config = ProverConfig::from_env();
        assert_eq!(config.private_key, Some("fallback_key".to_string()));

        std::env::remove_var("SP1_PRIVATE_KEY");
    }

    #[test]
    fn test_prover_creation() {
        let config = ProverConfig::new(Some("key".to_string()), None, false);
        let prover = AmdAttestationProver::new(config);
        assert!(prover.config().has_network_key());
    }

    #[tokio::test]
    async fn test_invalid_report_size() {
        let prover = AmdAttestationProver::from_env();
        let invalid_report = vec![0u8; 100]; // Wrong size

        let result = prover.prove(&invalid_report).await;
        assert!(result.is_err());

        match result {
            Err(Error::InvalidReportSize { expected, actual }) => {
                assert_eq!(expected, ATTESTATION_REPORT_SIZE);
                assert_eq!(actual, 100);
            }
            _ => panic!("Expected InvalidReportSize error"),
        }
    }
}
