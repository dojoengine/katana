use katana_metrics::Metrics;
use metrics::{Counter, Histogram};

#[derive(Metrics)]
#[metrics(scope = "block_producer")]
pub struct BlockProducerMetrics {
    /// The amount of L1 gas processed in a block.
    pub l1_gas_processed_total: Counter,
    /// The amount of Cairo steps processed in a block.
    pub cairo_steps_processed_total: Counter,
}

#[derive(Metrics)]
#[metrics(scope = "transaction_miner")]
pub struct TransactionMinerMetrics {
    /// Internal pool→miner handoff latency: time between transaction being added to pool
    /// and being picked up by the miner. Should typically be <10ms since miner is notified
    /// immediately via subscription. High values indicate internal performance issues
    /// (lock contention, notification delays, etc.).
    pub tx_pool_hand_off_time_seconds: Histogram,
    /// The number of transactions picked up from the pool by the miner.
    pub tx_pool_transactions_mined_total: Counter,
}
