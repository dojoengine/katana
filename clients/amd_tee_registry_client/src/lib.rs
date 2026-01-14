//! AMD TEE Attestation Prover
//!
//! Generate SP1 Groth16 proofs from AMD SEV-SNP attestation reports
//! for on-chain verification.
//!
//! ## Quick Start
//!
//! ```no_run
//! use amd_tee_registry_client::{AmdAttestationProver, ProverConfig};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create prover (reads NETWORK_PRIVATE_KEY from environment)
//! let prover = AmdAttestationProver::from_env();
//!
//! // Generate proof from attestation report bytes (1184 bytes)
//! let report_bytes: Vec<u8> = vec![/* ... */];
//! let proof = prover.prove(&report_bytes).await?;
//!
//! println!("Verifier ID: {}", proof.program_id.verifier_id);
//! # Ok(())
//! # }
//! ```
//!
//! ## Environment Variables
//!
//! - `SP1_PROVER`: Prover mode - "mock", "cpu", or "network"
//! - `NETWORK_PRIVATE_KEY`: Private key for SP1 Prover Network
//! - `SKIP_TIME_VALIDITY_CHECK`: Skip certificate time validation

pub mod error;
pub mod prover;

pub use error::Error;
pub use prover::{AmdAttestationProver, OnchainProof, ProverConfig, ATTESTATION_REPORT_SIZE};
