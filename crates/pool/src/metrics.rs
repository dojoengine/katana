use katana_metrics::Metrics;
use metrics::{Counter, Gauge};

/// Metrics for tracking transaction pool operations and state.
/// These track pool management operations, not end-to-end transaction latency.
#[derive(Metrics)]
#[metrics(scope = "tx_pool")]
pub struct PoolMetrics {
    /// The number of transactions successfully added to the pool after validation.
    pub transactions_added_total: Counter,
    /// The number of transactions removed from the pool (typically after block inclusion).
    pub transactions_removed_total: Counter,
    /// The current number of transactions waiting in the pool.
    pub transactions_current: Gauge,
    /// The number of transactions rejected during validation (invalid signature, nonce, etc.).
    pub transactions_rejected_total: Counter,
}
