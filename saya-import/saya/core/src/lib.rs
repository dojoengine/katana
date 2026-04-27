//! # Saya
//!
//! Saya is the proving orchestrator of the Dojo stack. The `saya-core` crate provides primitive
//! types and other constructs for embedding Saya into other applications. Refer to the `saya` crate
//! for the executable bindary.

/// Block ingestor abstraction and built-in implementations.
pub mod block_ingestor;

/// TEE (Trusted Execution Environment) pipeline stages and types.
pub mod tee;

/// Prover abstraction and built-in implementations.
pub mod prover;

/// Storage backend abstraction and built-in implementations.
pub mod storage;

/// Data availability backend abstraction and built-in implementations.
pub mod data_availability;

/// Base layer settlement provider abstraction and built-in implementations.
pub mod settlement;

/// Orchestrators for executing different rollup modes.
pub mod orchestrator;

/// Types related to handling long-running background services.
pub mod service;

/// Shared utilities (retry helpers).
pub mod utils;

pub use starknet_api::core::ChainId;
