use thiserror::Error;

/// Errors that can occur during TEE operations.
#[derive(Debug, Error)]
pub enum TeeError {
    /// I/O error when interacting with TEE interfaces.
    #[error("TEE I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Quote generation failed.
    #[error("Quote generation failed: {0}")]
    GenerationFailed(String),

    /// TEE functionality is not supported on this platform.
    #[error("TEE not supported: {0}")]
    NotSupported(String),

    /// Invalid report data size (must be exactly 64 bytes).
    #[error("Invalid report data size: expected 64 bytes, got {0}")]
    InvalidReportDataSize(usize),
}
