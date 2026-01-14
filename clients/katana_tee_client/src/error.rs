//! Error types for Katana TEE Client

use thiserror::Error;

/// Errors that can occur in the Katana TEE Client.
#[derive(Error, Debug)]
pub enum Error {
    /// I/O error
    #[error("IO error: {0}")]
    Io(String),

    /// JSON parsing error
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// Hex decoding error
    #[error("Hex decode error: {0}")]
    HexDecode(String),

    /// Attestation report parsing error
    #[error("Attestation report error: {0}")]
    AttestationReport(String),

    /// Prover error
    #[error("Prover error: {0}")]
    Prover(String),

    /// Certificate error
    #[error("Certificate error: {0}")]
    Certificate(String),

    /// RPC error
    #[error("RPC error: {0}")]
    Rpc(String),
}
