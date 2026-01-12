//! AMD TEE Registry Client
//!
//! This crate interfaces with AMD public APIs to retrieve the necessary
//! certificates for TEE attestation and verification.
//!
//! ## AMD Key Distribution Service (KDS)
//!
//! Base URL: `https://kdsintf.amd.com`
//!
//! ### Endpoints:
//! - `GET /vcek/v1/{processor}/cert_chain` - Returns ARK + ASK in PEM format
//! - `GET /vcek/v1/{processor}/{chip_id}?blSPL=X&teeSPL=X&snpSPL=X&ucodeSPL=X` - Returns VCEK in DER format
//!
//! ⚠️ VLEK is NOT available on KDS - only from the device!
//!
//! ## Related Resources
//! - [virtee/sev](https://github.com/virtee/sev) - Rust SEV/SNP implementation
//! - [google/go-sev-guest](https://github.com/google/go-sev-guest) - Go SEV-SNP library with test data
//! - [AMD KDS Specification](https://docs.amd.com) - Official AMD documentation

pub mod attestation;
pub mod error;
pub mod kds;
pub mod testdata;
pub mod types;

pub use attestation::AttestationReport;
pub use error::Error;
pub use kds::KdsClient;
pub use types::*;
