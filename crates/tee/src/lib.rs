//! TEE (Trusted Execution Environment) support for Katana.
//!
//! This crate provides abstractions for generating hardware-backed attestation
//! quotes that can cryptographically bind application state to a TEE measurement.
//!
//! # Supported TEE Platforms
//!
//! - **TDX (Intel Trust Domain Extensions)**: Via Linux ConfigFS-TSM interface
//! - **Mock**: For testing on non-TEE hardware (requires `mock` feature)
//!
//! # Example
//!
//! ```rust,ignore
//! use katana_tee::{TdxProvider, TeeProvider};
//!
//! let provider = TdxProvider::new()?;
//! let user_data = [0u8; 64]; // Your 64-byte commitment
//! let quote = provider.generate_quote(&user_data)?;
//! ```

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

mod error;
mod provider;
mod tdx;

#[cfg(any(test, feature = "mock"))]
mod mock;

pub use error::TeeError;
pub use provider::TeeProvider;
pub use tdx::TdxProvider;

#[cfg(any(test, feature = "mock"))]
pub use mock::MockProvider;

/// TEE provider type enumeration for CLI parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TeeProviderType {
    /// Intel TDX provider.
    #[default]
    Tdx,
    /// Mock provider for testing (only available with mock feature).
    #[cfg(any(test, feature = "mock"))]
    Mock,
}

impl std::fmt::Display for TeeProviderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tdx => write!(f, "tdx"),
            #[cfg(any(test, feature = "mock"))]
            Self::Mock => write!(f, "mock"),
        }
    }
}

impl std::str::FromStr for TeeProviderType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "tdx" => Ok(Self::Tdx),
            #[cfg(any(test, feature = "mock"))]
            "mock" => Ok(Self::Mock),
            other => Err(format!("Unknown TEE provider type: {}", other)),
        }
    }
}
