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

use crate::{
    config::ProverConfig, report::AttestationReportBytes, starknet::StarknetRegistryClient, Error,
};
use alloy_primitives::Bytes;
use amd_sev_snp_attestation_prover::{
    AmdSevSnpProver, ProverConfig as SdkProverConfig, RawProofType, SP1ProverConfig, KDS,
};
use amd_sev_snp_attestation_verifier::{stub::ProcessorType, AttestationReport};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, info};
use x509_verifier_rust_crypto::CertChain;

// Re-export OnchainProof for convenience
pub use amd_sev_snp_attestation_prover::OnchainProof;

/// Trait for SP1 proof generation.
///
/// This trait enables mocking the SP1 prover in tests.
pub trait Sp1Backend: Send + Sync {
    /// Generate an SP1 Groth16 proof from attestation report bytes.
    fn prove(
        &self,
        report_bytes: &[u8],
        timestamp: u64,
        config: &ProverConfig,
    ) -> Result<OnchainProof, Error>;
}

/// Default SP1 backend using the real SP1 SDK.
#[derive(Debug, Clone, Default)]
pub struct Sp1NetworkBackend;

impl Sp1Backend for Sp1NetworkBackend {
    fn prove(
        &self,
        report_bytes: &[u8],
        timestamp: u64,
        config: &ProverConfig,
    ) -> Result<OnchainProof, Error> {
        use amd_sev_snp_attestation_prover::{
            AmdSevSnpProver, ProverConfig as SdkProverConfig, SP1ProverConfig,
        };

        let sp1_config = SP1ProverConfig {
            private_key: config.private_key.clone(),
            rpc_url: config.rpc_url.clone(),
        };
        let mut sdk_config = SdkProverConfig::sp1_with(sp1_config);
        sdk_config.skip_time_validity_check = config.skip_time_validity_check;

        let prover = AmdSevSnpProver::new(sdk_config, None);
        prover
            .prove_attestation_report(timestamp, Bytes::from(report_bytes.to_vec()), None)
            .map_err(|e| Error::Prover(format!("Proof generation failed: {}", e)))
    }
}

/// AMD SEV-SNP Attestation Prover
///
/// Generates SP1 Groth16 proofs from AMD SEV-SNP attestation reports.
/// These proofs can be verified on-chain to prove TEE attestation.
#[derive(Debug, Clone)]
pub struct AmdAttestationProver<B: Sp1Backend = Sp1NetworkBackend> {
    config: ProverConfig,
    backend: B,
}

impl AmdAttestationProver<Sp1NetworkBackend> {
    /// Create a new prover with the given configuration.
    pub fn new(config: ProverConfig) -> Self {
        Self {
            config,
            backend: Sp1NetworkBackend,
        }
    }

    /// Create a prover with configuration from environment variables.
    pub fn from_env() -> Self {
        Self::new(ProverConfig::from_env())
    }
}

impl<B: Sp1Backend> AmdAttestationProver<B> {
    /// Create a prover with a custom backend (for testing).
    pub fn with_backend(config: ProverConfig, backend: B) -> Self {
        Self { config, backend }
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
        let report = AttestationReportBytes::new(report_bytes)?;

        info!("Starting SP1 proof generation for attestation report");
        debug!(
            "Report size: {} bytes, timestamp: {}",
            report.as_bytes().len(),
            timestamp
        );

        // Clone data for the blocking task
        let report_bytes = report.as_bytes().to_vec();
        let sdk_config = self.sdk_prover_config();

        // Run the blocking prover in a separate thread
        let proof = tokio::task::spawn_blocking(move || {
            // Create prover with SP1 configuration
            let prover = AmdSevSnpProver::new(sdk_config, None);

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

    /// Generate an SP1 Groth16 proof using Starknet cache information.
    ///
    /// This queries the on-chain cache for the trusted certificate prefix length
    /// and injects it into the verifier input before proving.
    pub async fn prove_with_cache(
        &self,
        report_bytes: &[u8],
        registry_client: &StarknetRegistryClient,
    ) -> Result<OnchainProof, Error> {
        self.prove_with_cache_and_timestamp(report_bytes, current_timestamp()?, registry_client)
            .await
    }

    /// Generate an SP1 Groth16 proof using Starknet cache information and a fixed timestamp.
    pub async fn prove_with_cache_and_timestamp(
        &self,
        report_bytes: &[u8],
        timestamp: u64,
        registry_client: &StarknetRegistryClient,
    ) -> Result<OnchainProof, Error> {
        let report = AttestationReportBytes::new(report_bytes)?;
        let report_struct = AttestationReport::from_bytes(report.as_bytes())
            .map_err(|e| Error::Prover(format!("Report parse failed: {e}")))?;

        let processor_model = report_struct
            .get_cpu_codename()
            .map_err(|e| Error::Prover(format!("Processor model error: {e}")))?;
        let processor_model_u8 = processor_type_to_u8(processor_model)?;

        let kds_chain = KDS::new()
            .fetch_report_cert_chain(&report_struct)
            .map_err(|e| Error::Prover(format!("KDS fetch failed: {e}")))?;
        let cert_chain = CertChain::parse_rev(&kds_chain)
            .map_err(|e| Error::Prover(format!("Cert chain parse failed: {e}")))?;

        let trusted_prefix_len = registry_client
            .fetch_trusted_prefix_len(processor_model_u8, cert_chain.digest())
            .await?;

        let report_bytes = report.as_bytes().to_vec();
        let sdk_config = self.sdk_prover_config();

        let proof = tokio::task::spawn_blocking(move || {
            let prover = AmdSevSnpProver::new(sdk_config, None);
            let mut input = prover
                .prepare_verifier_input(timestamp, Bytes::from(report_bytes), Some(kds_chain))
                .map_err(|e| Error::Prover(format!("Verifier input error: {e}")))?;
            input.trustedCertsPrefixLen = trusted_prefix_len;

            let raw_proof = prover
                .verifier
                .gen_proof(&input, RawProofType::Groth16, None)
                .map_err(|e| Error::Prover(format!("Proof generation failed: {e}")))?;
            prover
                .create_onchain_proof(raw_proof)
                .map_err(|e| Error::Prover(format!("Onchain proof error: {e}")))
        })
        .await
        .map_err(|e| Error::Prover(format!("Task join error: {}", e)))??;

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

    /// Build the SDK prover configuration from the explicit config.
    fn sdk_prover_config(&self) -> SdkProverConfig {
        let sp1_config = SP1ProverConfig {
            private_key: self.config.private_key.clone(),
            rpc_url: self.config.rpc_url.clone(),
        };
        let mut config = SdkProverConfig::sp1_with(sp1_config);
        config.skip_time_validity_check = self.config.skip_time_validity_check;
        config
    }
}

fn processor_type_to_u8(value: ProcessorType) -> Result<u8, Error> {
    let result = match value {
        ProcessorType::Milan => 0,
        ProcessorType::Genoa => 1,
        ProcessorType::Bergamo => 2,
        ProcessorType::Siena => 3,
        _ => {
            return Err(Error::Prover(format!(
                "Unsupported processor model: {value:?}"
            )))
        }
    };
    Ok(result)
}

/// Get the current Unix timestamp.
fn current_timestamp() -> Result<u64, Error> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|e| Error::Prover(format!("Failed to get timestamp: {}", e)))
}
