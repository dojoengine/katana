//! Versioned Starknet JSON-RPC API trait definitions.
//!
//! Each version module defines the same set of traits (`StarknetApi`, `StarknetWriteApi`,
//! `StarknetTraceApi`) using version-specific response types. The jsonrpsee `#[rpc]` macro
//! generates `*Server` traits (`StarknetApiServer`, etc.) that can be implemented independently
//! for each version.
//!
//! Spec: <https://github.com/starkware-libs/starknet-specs>

pub mod v0_9;
pub mod v0_10;

// Backward-compatible re-exports: default to v0.9 (the current spec version).
pub use v0_9::*;
