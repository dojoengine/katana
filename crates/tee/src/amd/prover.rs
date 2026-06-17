//! AMD SEV-SNP Attestation SP1 Prover
//!
//! This module provides functionality to generate SP1 Groth16 proofs
//! from AMD SEV-SNP attestation reports with on-chain cache lookup.
//!
//! # Overview
//!
//! The prover takes a raw AMD SEV-SNP attestation report (1184 bytes),
//! queries the Starknet registry for cached certificate state, and
//! generates a zero-knowledge proof that can be verified on-chain.
//!
//! # Proof Types
//!
//! - **Groth16**: Compact proofs (~260 bytes) for on-chain verification
//!
//! # Usage
//!
//! ```no_run
//! use amd_tee_registry_client::{AmdAttestationProver, ProverConfig, StarknetRegistryClient};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create prover and registry client
//! let prover = AmdAttestationProver::new(ProverConfig::from_env());
//! let registry = StarknetRegistryClient::new("https://rpc.url", 0x123.into());
//!
//! // Raw attestation report bytes (1184 bytes)
//! let report_bytes: Vec<u8> = vec![/* ... */];
//!
//! // Generate Groth16 proof with cache lookup
//! let proof_info = prover.prove(&report_bytes, &registry).await?;
//!
//! println!("Proof generated: {} bytes", proof_info.proof.onchain_proof.len());
//! println!("Trusted prefix len: {}", proof_info.trusted_prefix_len);
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

use std::time::{SystemTime, UNIX_EPOCH};

use alloy_primitives::{Bytes, B256};
// Re-export OnchainProof for convenience
pub use amd_sev_snp_attestation_prover::OnchainProof;
use amd_sev_snp_attestation_prover::{
    AmdSevSnpProver, ProverConfig as SdkProverConfig, RawProofType, SP1ProverConfig, KDS,
};
use amd_sev_snp_attestation_verifier::stub::{ProcessorType, VerifierInput};
use amd_sev_snp_attestation_verifier::AttestationReport;
use katana_tracing::info;
use x509_verifier_rust_crypto::CertChain;

use crate::amd::config::ProverConfig;
use crate::amd::report::AttestationReportBytes;
use crate::amd::starknet::StarknetRegistryClient;
use crate::amd::Error;

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

/// Optional storage proof data for ZK circuit. When set, the circuit verifies:
/// 1. global_state_root = hash("STARKNET_STATE_V0", contracts_tree_root, classes_tree_root)
/// 2. Contract with storage_root is in contracts_tree at contract_address
/// 3. Storage keys/values are in contract's storage trie
///
/// Then commits poseidon_hash(storage_commitment, contract_address, nonce, global_state_root).
#[derive(Debug, Clone, Default)]
pub struct StorageProofParams {
    // Global state verification (matches attestation)
    pub global_state_root: B256,
    pub contracts_tree_root: B256,
    pub classes_tree_root: B256,
    // Contracts tree proof
    pub contracts_proof_nodes: Vec<Bytes>,
    pub contract_storage_root: B256,
    pub contract_class_hash: B256,
    pub contract_leaf_nonce: u64,
    // Storage proof
    pub keys: Vec<Bytes>,
    pub values: Vec<Bytes>,
    pub storage_proof_nodes: Vec<Bytes>,
    // Replay protection
    pub contract_address: B256,
    pub nonce: u64,
    // Fork block number (0 = non-fork mode)
    pub fork_block_number: u64,
}

/// Parameters for event inclusion proof (C2: proves shard ending).
/// Verifies a specific event is in the block's events_commitment Merkle root.
#[derive(Debug, Clone)]
pub struct EventProofParams {
    pub events_commitment: B256,
    pub event_hash: B256,
    pub event_index: u32,
    pub events_count: u32,
    pub event_merkle_proof: Vec<Bytes>,
    pub end_block_number: u64,
}

/// Proof result with cache metadata for transparency
#[derive(Debug)]
pub struct ProofWithCacheInfo {
    /// The generated SP1 proof
    pub proof: OnchainProof,
    /// How many certs were trusted on-chain at proof time
    pub trusted_prefix_len: u8,
    /// Certificate chain digests [root, ask, vcek]
    pub cert_digests: Vec<[u8; 32]>,
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
            prover_mode: None,
        };

        let mut sdk_config = SdkProverConfig::sp1_with(sp1_config);
        sdk_config.skip_time_validity_check = config.skip_time_validity_check;

        AmdSevSnpProver::new(sdk_config, None)
            .prove_attestation_report(timestamp, Bytes::from(report_bytes.to_vec()), None)
            .map_err(|e| Error::Prover(format!("Proof generation failed: {e}")))
    }
}

/// AMD SEV-SNP Attestation Prover
///
/// Generates SP1 Groth16 proofs from AMD SEV-SNP attestation reports.
/// These proofs can be verified on-chain to prove TEE attestation.
#[derive(Debug, Clone)]
pub struct AmdAttestationProver<B: Sp1Backend = Sp1NetworkBackend> {
    config: ProverConfig,
    #[allow(dead_code)] // Backend trait is for future testability
    backend: B,
}

impl AmdAttestationProver<Sp1NetworkBackend> {
    /// Create a new prover with the given configuration.
    pub fn new(config: ProverConfig) -> Self {
        Self { config, backend: Sp1NetworkBackend }
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

    /// Generate an SP1 Groth16 proof with on-chain cache lookup.
    ///
    /// # Flow
    /// 1. Parse report to get processor model
    /// 2. Fetch cert chain from AMD KDS
    /// 3. Query Starknet registry for trusted prefix length
    /// 4. Build verifier input with correct prefix_len
    /// 5. Generate SP1 Groth16 proof
    ///
    /// # Arguments
    /// * `report_bytes` - Raw 1184-byte attestation report
    /// * `registry_client` - Starknet AMDTeeRegistry client (required)
    pub async fn prove(
        &self,
        report_bytes: &[u8],
        registry_client: &StarknetRegistryClient,
    ) -> Result<ProofWithCacheInfo, Error> {
        self.prove_with_timestamp(report_bytes, current_timestamp()?, registry_client).await
    }

    /// Generate an SP1 Groth16 proof with optional storage proof (same flow as
    /// `prove_with_timestamp`). When `storage` is `Some`, the ZK circuit verifies the proof and
    /// commits storage commitment to the journal.
    pub async fn prove_with_storage(
        &self,
        report_bytes: &[u8],
        registry_client: &StarknetRegistryClient,
        storage: Option<StorageProofParams>,
        event_proof: Option<EventProofParams>,
    ) -> Result<ProofWithCacheInfo, Error> {
        self.prove_with_timestamp_and_storage(
            report_bytes,
            current_timestamp()?,
            registry_client,
            storage,
            event_proof,
        )
        .await
    }

    /// Generate an SP1 Groth16 proof with a specific timestamp and cache lookup.
    pub async fn prove_with_timestamp(
        &self,
        report_bytes: &[u8],
        timestamp: u64,
        registry_client: &StarknetRegistryClient,
    ) -> Result<ProofWithCacheInfo, Error> {
        self.prove_with_timestamp_and_storage(report_bytes, timestamp, registry_client, None, None)
            .await
    }

    /// Generate an SP1 Groth16 proof with timestamp, cache lookup, and optional storage/event
    /// proofs.
    pub async fn prove_with_timestamp_and_storage(
        &self,
        report_bytes: &[u8],
        timestamp: u64,
        registry_client: &StarknetRegistryClient,
        storage: Option<StorageProofParams>,
        event_proof: Option<EventProofParams>,
    ) -> Result<ProofWithCacheInfo, Error> {
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

        let cert_digests: Vec<[u8; 32]> = cert_chain.digest().iter().map(|d| d.0).collect();

        let trusted_prefix_len = registry_client
            .fetch_trusted_prefix_len(processor_model_u8, cert_chain.digest())
            .await?;

        info!(
            "Cache state: {} of {} certs trusted on-chain",
            trusted_prefix_len,
            cert_digests.len()
        );

        // Validate cert chain time if not skipped
        if !self.config.skip_time_validity_check {
            cert_chain
                .check_valid(timestamp)
                .map_err(|e| Error::Prover(format!("Cert chain time validation failed: {e}")))?;
        }

        let report_bytes = report.as_bytes().to_vec();
        let sdk_config = self.sdk_prover_config();
        let vek_der_chain = cert_chain.to_ders();

        let proof = tokio::task::spawn_blocking(move || {
            let prover = AmdSevSnpProver::new(sdk_config, None);

            let input = prepare_verifier_input_with_storage(
                timestamp,
                Bytes::from(report_bytes),
                vek_der_chain,
                trusted_prefix_len,
                storage,
                event_proof,
            );

            let raw_proof = prover
                .verifier
                .gen_proof(&input, RawProofType::Groth16, None)
                .map_err(|e| Error::Prover(format!("Proof generation failed: {e}")))?;
            prover
                .create_onchain_proof(raw_proof)
                .map_err(|e| Error::Prover(format!("Onchain proof error: {e}")))
        })
        .await
        .map_err(|e| Error::Prover(format!("Task join error: {e}")))??;

        Ok(ProofWithCacheInfo { proof, trusted_prefix_len, cert_digests })
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
            prover_mode: None,
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
        _ => return Err(Error::Prover(format!("Unsupported processor model: {value:?}"))),
    };
    Ok(result)
}

/// Builds `VerifierInput` for attestation; optionally attaches storage proof data.
/// Storage logic lives here (and in callers like sharding_operator), not in the SDK prover.
pub fn prepare_verifier_input_with_storage(
    timestamp: u64,
    raw_report: Bytes,
    vek_der_chain: Vec<Bytes>,
    trusted_certs_prefix_len: u8,
    storage: Option<StorageProofParams>,
    event_proof: Option<EventProofParams>,
) -> VerifierInput {
    let mut input = VerifierInput {
        timestamp,
        trustedCertsPrefixLen: trusted_certs_prefix_len,
        rawReport: raw_report,
        vekDerChain: vek_der_chain,
        // Global state verification
        globalStateRoot: B256::ZERO,
        contractsTreeRoot: B256::ZERO,
        classesTreeRoot: B256::ZERO,
        // Contracts tree proof
        contractsProofNodes: vec![],
        contractStorageRoot: B256::ZERO,
        contractClassHash: B256::ZERO,
        contractLeafNonce: 0,
        // Storage proof
        storageKeys: vec![],
        storageValues: vec![],
        storageProofNodes: vec![],
        // Replay protection
        contractAddress: B256::ZERO,
        nonce: 0,
        // Fork block (set by caller if in fork mode)
        forkBlockNumber: 0,
        // Event proof (empty = no event proof)
        eventsCommitment: B256::ZERO,
        eventHash: B256::ZERO,
        eventIndex: 0,
        eventsCount: 0,
        eventMerkleProof: vec![],
        endBlockNumber: 0,
    };

    if let Some(s) = storage {
        input.globalStateRoot = s.global_state_root;
        input.contractsTreeRoot = s.contracts_tree_root;
        input.classesTreeRoot = s.classes_tree_root;
        input.contractsProofNodes = s.contracts_proof_nodes;
        input.contractStorageRoot = s.contract_storage_root;
        input.contractClassHash = s.contract_class_hash;
        input.contractLeafNonce = s.contract_leaf_nonce;
        input.storageKeys = s.keys;
        input.storageValues = s.values;
        input.storageProofNodes = s.storage_proof_nodes;
        input.contractAddress = s.contract_address;
        input.nonce = s.nonce;
        input.forkBlockNumber = s.fork_block_number;
    }

    if let Some(e) = event_proof {
        input.eventsCommitment = e.events_commitment;
        input.eventHash = e.event_hash;
        input.eventIndex = e.event_index;
        input.eventsCount = e.events_count;
        input.eventMerkleProof = e.event_merkle_proof;
        input.endBlockNumber = e.end_block_number;
    }

    input
}

/// Get the current Unix timestamp.
fn current_timestamp() -> Result<u64, Error> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|e| Error::Prover(format!("Failed to get timestamp: {e}")))
}
