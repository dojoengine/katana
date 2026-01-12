//! Error types for AMD TEE Registry Client

use thiserror::Error;

/// Errors that can occur when interacting with AMD KDS
#[derive(Error, Debug)]
pub enum Error {
    /// HTTP request failed
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    /// Certificate parsing failed
    #[error("Certificate parsing failed: {0}")]
    CertificateParse(String),

    /// PEM parsing failed
    #[error("PEM parsing failed: {0}")]
    PemParse(#[from] pem::PemError),

    /// DER parsing failed
    #[error("DER parsing failed: {0}")]
    DerParse(String),

    /// Invalid processor type
    #[error("Invalid processor type: {0}")]
    InvalidProcessorType(String),

    /// Invalid chip ID
    #[error("Invalid chip ID: expected 64 bytes, got {0}")]
    InvalidChipId(usize),

    /// KDS returned an error response
    #[error("KDS error: {status} - {message}")]
    KdsError { status: u16, message: String },
}
