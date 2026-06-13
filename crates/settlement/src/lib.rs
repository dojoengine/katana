//! Embedded settlement service.
//!
//! Settles the node's own blocks to a core contract on a settlement chain.
//! The service is generic over the two axes of a settlement setup:
//!
//! - **How** a state transition is proven: a [`backend::ProvingBackend`] turns each unsettled block
//!   range into a state-update payload. [`TeeBackend`] (AMD SEV-SNP attestations proven with SP1
//!   Groth16, or mock journals under a mock attester) is the only proving system today; standard
//!   validity proofs (e.g. SNOS + STARK) would be a second backend.
//! - **Where** it is settled: a [`chain::SettlementChain`] owns the RPC client, settlement account,
//!   and core contract binding. [`StarknetSettlementChain`] (the Piltover core contract on a
//!   Starknet chain) is the only chain today; settling to Ethereum would be a second implementation
//!   with its own core contract and RPC client.
//!
//! A backend and a chain are paired in a [`SettlementPipeline`], which
//! statically checks that the backend's payload type is what the chain's
//! core contract accepts. The service owns the rest — the serial settle
//! loop, batching, the on-chain progress cursor, retries, and transaction
//! submission.
//!
//! Progress tracking is fully on-chain: Piltover's `get_state()` is the only
//! durable cursor. On (re)start the service reads it and drains the backlog
//! from the local provider; afterwards the node's block broadcast is used as a
//! low-latency wake-up only — every iteration recomputes from durable state,
//! so missed or lagged notifications are harmless.

pub mod backend;
pub mod chain;
mod config;
mod service;

pub use backend::tee::{TeeBackend, TeeProver};
pub use backend::ProvingBackend;
pub use chain::starknet::StarknetSettlementChain;
pub use chain::SettlementChain;
pub use config::{ChainConfig, ProverConfig, SettlementConfig};
pub use service::{SettlementPipeline, SettlementService, SettlementServiceHandle};

pub(crate) const LOG_TARGET: &str = "settlement";
