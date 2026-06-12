//! Embedded TEE settlement service.
//!
//! Settles the node's own blocks to the Piltover core contract on the
//! settlement chain: for each batch of blocks it builds a TEE attestation
//! (same code path as the `tee_generateQuote` RPC), turns it into an
//! `sp1_proof` payload (a real SP1 Groth16 proof, or a mock journal when the
//! node runs a mock attester), and submits `update_state(TeeInput)`.
//!
//! Progress tracking is fully on-chain: Piltover's `get_state()` is the only
//! durable cursor. On (re)start the service reads it and drains the backlog
//! from the local provider; afterwards the node's block broadcast is used as a
//! low-latency wake-up only — every iteration recomputes from durable state,
//! so missed or lagged notifications are harmless.

mod config;
mod mock;
mod piltover;
mod prover;
mod service;

pub use config::SettlementConfig;
pub use prover::TeeProver;
pub use service::{SettlementService, SettlementServiceHandle};

pub(crate) const LOG_TARGET: &str = "settlement";
