//! Metrics for the settlement service.
//!
//! Instrumented at the service layer so they are agnostic to the proving
//! backend: `proof_generation_seconds` times the whole [`ProvingBackend::prove`]
//! call (attestation build + proving), regardless of whether it is the TEE/SP1
//! backend or a future validity-proof one.
//!
//! [`ProvingBackend::prove`]: crate::backend::ProvingBackend::prove

use katana_metrics::Metrics;
use metrics::{Counter, Gauge, Histogram};

/// Proof-related settlement metrics.
///
/// Constructed with a `proof_type` label (e.g. `sp1`, `mock`) identifying the
/// proving backend, so proof timings can be broken down and compared across
/// backends.
#[derive(Metrics)]
#[metrics(scope = "settlement")]
pub(crate) struct SettlementProofMetrics {
    /// Time spent generating a proof for a batch, in seconds — the backend
    /// `prove` call (attestation build + proving). This is the dominant cost of
    /// settlement and the primary thing to watch.
    pub(crate) proof_generation_seconds: Histogram,

    /// End-to-end time to settle a batch (prove + submit), in seconds.
    pub(crate) settle_batch_seconds: Histogram,
}

/// Metrics for the settlement service's settle loop.
#[derive(Metrics)]
#[metrics(scope = "settlement")]
pub(crate) struct SettlementMetrics {
    /// Time spent submitting the `update_state` transaction to the Piltover core
    /// contract and waiting for it to land, in seconds.
    pub(crate) state_update_seconds: Histogram,

    /// Number of blocks in each settled batch.
    pub(crate) blocks_per_batch: Histogram,

    /// Total number of batches successfully settled.
    pub(crate) batches_settled_total: Counter,

    /// Total number of blocks successfully settled.
    pub(crate) blocks_settled_total: Counter,

    /// Total number of failed settlement attempts.
    pub(crate) settlement_failures_total: Counter,

    /// The last block number successfully settled to the core contract.
    pub(crate) settled_block: Gauge,
}
