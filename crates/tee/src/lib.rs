//! TEE (Trusted Execution Environment) support for Katana.
//!
//! This crate provides abstractions for generating hardware-backed attestation
//! quotes that can cryptographically bind application state to a TEE measurement.
//!
//! # Supported TEE Platforms
//!
//! - **SEV-SNP (AMD Secure Encrypted Virtualization)**: Via Automata Network SDK (requires `snp`
//!   feature)
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

#[cfg(test)]
mod mock;

#[cfg(feature = "snp")]
mod snp;

pub use error::TeeError;
#[cfg(test)]
pub use mock::MockProvider;
pub use provider::TeeProvider;
#[cfg(feature = "snp")]
pub use snp::SevSnpProvider;

/// TEE provider type enumeration.
///
/// Currently only SEV-SNP is supported for production use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeeProviderType {
    /// AMD SEV-SNP provider.
    SevSnp,
}

impl std::fmt::Display for TeeProviderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SevSnp => write!(f, "sev-snp"),
        }
    }
}

impl std::str::FromStr for TeeProviderType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "sev-snp" | "snp" => Ok(Self::SevSnp),
            other => {
                Err(format!("Unknown TEE provider: '{}'. Available providers: sev-snp", other))
            }
        }
    }
}
