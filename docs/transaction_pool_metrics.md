# Transaction Pool Metrics

This document describes the metrics tracking system for transaction pool internal performance in Katana.

## Overview

The transaction pool metrics system tracks internal system performance, specifically:
1. **Pool→Miner handoff latency**: How quickly the miner picks up transactions after they're added to the pool
2. **Pool management operations**: Transaction additions, removals, and validation outcomes

These metrics help operators monitor internal system health and identify performance bottlenecks in the transaction processing pipeline.

## Metrics Available

### Block Producer Metrics (scope: `block_producer`)

- **`tx_pool_residence_time_seconds`** (Histogram): Internal pool→miner handoff latency in seconds. Should typically be <0.010s (10ms) since the miner is notified immediately. High values indicate performance issues like lock contention or notification delays.
- **`tx_pool_transactions_mined_total`** (Counter): Total number of transactions picked up from the pool by the miner

### Pool Metrics (scope: `tx_pool`)

- **`transactions_added_total`** (Counter): Total number of transactions added to the pool
- **`transactions_removed_total`** (Counter): Total number of transactions removed from the pool
- **`transactions_current`** (Gauge): Current number of transactions in the pool
- **`transactions_rejected_total`** (Counter): Total number of transactions rejected due to validation errors

## Implementation Details

### Timing Mechanism

1. When a transaction is added to the pool, it gets a timestamp in the `PendingTx.added_at` field
2. The miner is immediately notified via subscription and should pick up the transaction quickly
3. When the `TransactionMiner` picks up transactions via `poll()`, it calculates handoff latency as `now - added_at`
4. This latency (should be <10ms) is recorded in the `tx_pool_residence_time_seconds` histogram

**Important**: This measures internal system performance, not end-to-end user transaction time.

### Code Locations

- Pool metrics: `crates/pool/src/metrics.rs`
- Block producer metrics: `crates/core/src/service/metrics.rs`
- Timing logic: `crates/core/src/service/mod.rs` in `TransactionMiner::poll()`
- Pool size tracking: `crates/pool/src/pool.rs` in `add_transaction()` and `remove_transactions()`

## Usage

These metrics are automatically collected when the node runs with metrics enabled. They can be exposed via:

1. **Prometheus endpoint**: Configure the metrics server to expose these metrics for scraping
2. **OpenTelemetry**: If OTLP tracing is enabled, these metrics are included in telemetry data
3. **Logs**: Metrics can be logged for debugging purposes

## Example Queries

### Prometheus/PromQL

```promql
# Average transaction residence time over 5 minutes
rate(block_producer_tx_pool_residence_time_seconds_sum[5m]) / rate(block_producer_tx_pool_residence_time_seconds_count[5m])

# 95th percentile residence time
histogram_quantile(0.95, rate(block_producer_tx_pool_residence_time_seconds_bucket[5m]))

# Current pool size
tx_pool_transactions_current

# Transaction throughput (transactions per second)
rate(block_producer_tx_pool_transactions_mined_total[1m])
```

This system provides comprehensive observability into transaction pool performance and helps identify bottlenecks in transaction processing.
