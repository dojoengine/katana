//! TEE (Trusted Execution Environment) support for Katana.
//!
//! This crate provides abstractions for generating hardware-backed attestation
//! quotes that can cryptographically bind application state to a TEE measurement.
//!
//! # Supported TEE Platforms
//!
//! - **SEV-SNP (AMD Secure Encrypted Virtualization)**: Via Automata Network SDK (requires `snp`
//!   feature)
//! - **Mock**: For testing on non-TEE hardware (requires `mock` feature)
//!
//! # Example
//!
//! ```rust,ignore
//! use katana_tee::{SevSnpProvider, TeeProvider};
//!
//! let provider = SevSnpProvider::new()?;
//! let user_data = [0u8; 64]; // Your 64-byte commitment
//! let quote = provider.generate_quote(&user_data)?;
//! ```

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

mod error;
mod provider;

#[cfg(any(test, feature = "mock"))]
mod mock;

#[cfg(feature = "snp")]
mod snp;

pub use error::TeeError;
#[cfg(any(test, feature = "mock"))]
pub use mock::MockProvider;
pub use provider::TeeProvider;
#[cfg(feature = "snp")]
pub use snp::SevSnpProvider;

/// TEE provider type enumeration for CLI parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeeProviderType {
    /// AMD SEV-SNP provider (only available with snp feature).
    #[cfg(feature = "snp")]
    SevSnp,
    /// Mock provider for testing (only available with mock feature).
    #[cfg(any(test, feature = "mock"))]
    Mock,
}

impl Default for TeeProviderType {
    fn default() -> Self {
        #[cfg(feature = "snp")]
        {
            Self::SevSnp
        }
        #[cfg(not(feature = "snp"))]
        {
            Self::Mock
        }
    }
}

impl std::fmt::Display for TeeProviderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(feature = "snp")]
            Self::SevSnp => write!(f, "sev-snp"),
            #[cfg(any(test, feature = "mock"))]
            Self::Mock => write!(f, "mock"),
        }
    }
}

impl std::str::FromStr for TeeProviderType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            #[cfg(feature = "snp")]
            "sev-snp" | "snp" => Ok(Self::SevSnp),
            #[cfg(any(test, feature = "mock"))]
            "mock" => Ok(Self::Mock),
            other => Err(format!("Unknown TEE provider type: {}", other)),
        }
    }
}
