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

pub mod calldata;
pub mod error;
pub mod prover;

pub use calldata::StarknetCalldata;
pub use error::Error;
pub use prover::{AmdAttestationProver, OnchainProof, ProverConfig, ATTESTATION_REPORT_SIZE};

// Re-export BigUint for users who need raw calldata values
pub use num_bigint::BigUint;
