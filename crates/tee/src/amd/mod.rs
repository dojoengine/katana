//! AMD TEE Attestation Prover
//!
//! Generate SP1 Groth16 proofs from AMD SEV-SNP attestation reports
//! and convert them to Starknet calldata for on-chain verification.
//!
//! ## Quick Start
//!
//! ```no_run
//! use amd_tee_registry_client::{AmdAttestationProver, ProverConfig, StarknetCalldata};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create prover (reads NETWORK_PRIVATE_KEY from environment)
//! let prover = AmdAttestationProver::from_env();
//!
//! // Generate proof from attestation report bytes (1184 bytes)
//! let report_bytes: Vec<u8> = vec![/* ... */];
//! let proof = prover.prove(&report_bytes).await?;
//!
//! // Convert to Starknet calldata
//! let calldata = StarknetCalldata::from_proof(&proof)?;
//!
//! // Get hex strings for contract calls
//! println!("Calldata elements: {}", calldata.len());
//! for hex in calldata.to_hex_strings() {
//!     println!("{}", hex);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Environment Variables
//!
//! - `SP1_PROVER`: Prover mode - "mock", "cpu", or "network"
//! - `NETWORK_PRIVATE_KEY`: Private key for SP1 Prover Network
//! - `SKIP_TIME_VALIDITY_CHECK`: Skip certificate time validation

pub mod cairo_fixtures;
pub mod config;
pub mod error;
pub mod kds;
pub mod prover;
pub mod report;
pub mod starknet;

pub use cairo_fixtures::generate_cairo_fixtures;
pub use config::ProverConfig;
pub use error::Error;
pub use kds::{KdsClient, RootCertFetcher, RootCertInfo};
pub use prover::{
    prepare_verifier_input_with_storage, AmdAttestationProver, EventProofParams, OnchainProof,
    ProofWithCacheInfo, Sp1Backend, Sp1NetworkBackend, StorageProofParams,
};
pub use report::ATTESTATION_REPORT_SIZE;
pub use starknet::StarknetRegistryClient;
