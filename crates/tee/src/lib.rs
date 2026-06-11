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
//! use katana_tee::{Attester, SevSnpAttester};
//!
//! let attester = SevSnpAttester::new()?;
//! let user_data = [0u8; 64]; // Your 64-byte commitment
//! let quote = attester.generate_quote(&user_data)?;
//! ```

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

// Used by the `kds-client` binary; silence the lib-only `unused_crate_dependencies` lint.
use anyhow as _;
use std::fmt::Debug;

// `zeroize` is used only by the `snp-derivekey` binary, not this library.
// Acknowledge the workspace dep here so `unused_crate_dependencies` stays quiet.
#[cfg(feature = "snp")]
use zeroize as _;

mod error;

#[cfg(any(test, feature = "mock"))]
pub mod mock;

#[cfg(feature = "snp")]
mod snp;

pub mod amd;

pub use error::TeeError;
#[cfg(any(test, feature = "mock"))]
pub use mock::MockAttester;
#[cfg(feature = "snp")]
pub use snp::SevSnpAttester;

/// Trait for TEE attesters that can generate attestation quotes.
///
/// Implementations of this trait interact with TEE hardware to generate
/// cryptographic attestation quotes that bind user-provided data to the
/// hardware state.
pub trait Attester: Send + Sync + Debug {
    /// Generate an attestation quote with the given user data.
    ///
    /// # Arguments
    /// * `user_data` - A 64-byte slice of user-provided data to include in the quote. This is
    ///   typically a hash commitment to application state.
    ///
    /// # Returns
    /// * `Ok(Vec<u8>)` - The raw attestation quote bytes.
    /// * `Err(TeeError)` - If quote generation fails.
    fn generate_quote(&self, user_data: &[u8; 64]) -> Result<Vec<u8>, TeeError>;

    /// Returns the name of this attester for logging purposes.
    fn name(&self) -> &'static str;
}

/// TEE attester kind enumeration.
///
/// `SevSnp` is the only variant intended for production use. `Mock` is gated
/// behind the `mock` feature and exists exclusively to let integration test
/// crates serve `tee_generateQuote` on machines without AMD SEV-SNP hardware.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, clap::ValueEnum,
)]
pub enum AttesterKind {
    /// AMD SEV-SNP attester.
    #[value(name = "sev-snp", alias = "snp")]
    SevSnp,
    /// Software-only mock attester (test infrastructure only).
    #[cfg(any(test, feature = "mock"))]
    #[value(name = "mock")]
    Mock,
}

impl std::fmt::Display for AttesterKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SevSnp => write!(f, "sev-snp"),
            #[cfg(any(test, feature = "mock"))]
            Self::Mock => write!(f, "mock"),
        }
    }
}

impl std::str::FromStr for AttesterKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "sev-snp" | "snp" => Ok(Self::SevSnp),
            #[cfg(any(test, feature = "mock"))]
            "mock" => Ok(Self::Mock),
            other => Err(format!("Unknown TEE attester: '{other}'. Available attesters: sev-snp")),
        }
    }
}
