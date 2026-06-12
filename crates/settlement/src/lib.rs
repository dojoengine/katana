//! Embedded settlement service.
//!
//! Settles the node's own blocks to the Piltover core contract on the
//! settlement chain. The service itself is generic over the proving system:
//! a [`backend::ProvingBackend`] turns each unsettled block range into the
//! `update_state` payload, while the service owns the proof-system-agnostic
//! machinery — the serial settle loop, batching, the on-chain progress
//! cursor, retries, and transaction submission.
//!
//! [`backend::tee::TeeBackend`] (AMD SEV-SNP attestations proven with SP1
//! Groth16, or mock journals under a mock attester) is the only proving
//! system today; standard validity proofs (e.g. SNOS + STARK) would slot in
//! as a second backend.
//!
//! Progress tracking is fully on-chain: Piltover's `get_state()` is the only
//! durable cursor. On (re)start the service reads it and drains the backlog
//! from the local provider; afterwards the node's block broadcast is used as a
//! low-latency wake-up only — every iteration recomputes from durable state,
//! so missed or lagged notifications are harmless.

pub mod backend;
mod config;
mod piltover;
mod service;

pub use backend::tee::{TeeBackend, TeeProver};
pub use backend::ProvingBackend;
pub use config::{ProverConfig, SettlementConfig};
pub use service::{SettlementService, SettlementServiceHandle};

pub(crate) const LOG_TARGET: &str = "settlement";
