//! Error types for AMD TEE Attestation Prover

/// Errors that can occur during AMD TEE attestation proving
#[derive(thiserror::Error, Debug)]
pub enum Error {
    // === Domain errors ===
    /// Invalid attestation report size
    #[error("Invalid attestation report size: expected {expected} bytes, got {actual}")]
    InvalidReportSize { expected: usize, actual: usize },

    /// Attestation report parsing/validation error
    #[error("Attestation report error: {0}")]
    AttestationReport(String),

    /// Certificate error
    #[error("Certificate error: {0}")]
    Certificate(String),

    // === Infrastructure errors ===
    /// Prover error
    #[error("Prover error: {0}")]
    Prover(String),

    /// Calldata generation error
    #[error("Calldata error: {0}")]
    Calldata(String),

    /// Starknet RPC error
    #[error("Starknet error: {0}")]
    Starknet(String),

    /// KDS (AMD Key Distribution Service) error
    #[error("KDS error: {0}")]
    Kds(String),

    // === Common errors (shared with katana_tee_client) ===
    /// Hex decode error
    #[error("Hex decode error: {0}")]
    HexDecode(String),

    /// JSON error
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// I/O error
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
