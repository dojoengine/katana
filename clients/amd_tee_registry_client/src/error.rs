//! Error types for AMD TEE Attestation Prover

use thiserror::Error;

/// Errors that can occur during AMD TEE attestation proving
#[derive(Error, Debug)]
pub enum Error {
    /// Invalid attestation report size
    #[error("Invalid attestation report size: expected {expected} bytes, got {actual}")]
    InvalidReportSize { expected: usize, actual: usize },

    /// Prover error
    #[error("Prover error: {0}")]
    Prover(String),

    /// Calldata generation error
    #[error("Calldata error: {0}")]
    Calldata(String),
}
