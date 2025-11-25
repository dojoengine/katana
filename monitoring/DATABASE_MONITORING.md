# Database Monitoring Guide

This guide explains the comprehensive database monitoring panels added to the Grafana dashboard for tracking MDBX database performance and operations.

## Overview

The database monitoring setup provides deep insights into:
- Transaction lifecycle and performance
- CRUD operation rates and success rates
- Operation timing and latency percentiles
- Cache hit rates and efficiency
- Database table statistics

## Metrics Source

All metrics are collected from the MDBX database layer in `katana-db` and exposed via the `/` metrics endpoint on port 9100 (configurable via `--metrics.port`).

## Dashboard Sections

### 1. Database Transactions

#### Transaction Creation Rate
- **Type**: Time series chart
- **Metrics**: 
  - `rate(katana_db_transaction_ro_created[5m])` - Read-only transactions per second
  - `rate(katana_db_transaction_rw_created[5m])` - Read-write transactions per second
- **Purpose**: Monitor the rate at which database transactions are being created
- **What to look for**: 
  - High RO transaction rates indicate heavy read operations
  - High RW transaction rates indicate heavy write operations
  - Sudden spikes may indicate performance issues or load changes

#### Transaction Commit Status
- **Type**: Time series chart
- **Metrics**:
  - `rate(katana_db_transaction_commits_successful[5m])` - Successful commits/sec
  - `rate(katana_db_transaction_commits_failed[5m])` - Failed commits/sec
  - `rate(katana_db_transaction_aborts[5m])` - Aborted transactions/sec
- **Purpose**: Track the health of transaction commits
- **What to look for**:
  - Failed commits or aborts should be rare
  - Increasing failure rates indicate potential database issues
  - Aborts may indicate transaction conflicts or timeouts

#### Transaction Totals (Stat Panels)
- **Total RO Transactions**: Cumulative read-only transactions
- **Total RW Transactions**: Cumulative read-write transactions
- **Successful Commits**: Total successful transaction commits
- **Failed/Aborted**: Total failed commits + aborted transactions

### 2. Database Operations

#### Operation Rate by Type
- **Type**: Time series chart
- **Metrics**:
  - `rate(katana_db_operation_puts[5m])` - Put operations/sec
  - `rate(katana_db_operation_get_hits[5m]) + rate(katana_db_operation_get_misses[5m])` - Total get operations/sec
  - `rate(katana_db_operation_deletes_successful[5m]) + rate(katana_db_operation_deletes_failed[5m])` - Total delete operations/sec
  - `rate(katana_db_operation_clears[5m])` - Clear operations/sec
- **Purpose**: Monitor the distribution and rate of different database operations
- **What to look for**:
  - Unbalanced operation rates may indicate application patterns
  - High clear operation rates may indicate excessive cleanup

#### Delete Success Rate
- **Type**: Time series chart
- **Metrics**:
  - `rate(katana_db_operation_deletes_successful[5m])` - Successful deletes/sec
  - `rate(katana_db_operation_deletes_failed[5m])` - Failed deletes/sec
- **Purpose**: Track delete operation reliability
- **What to look for**:
  - Failed deletes may indicate key-not-found scenarios (expected) or database issues (unexpected)

#### Get Cache Hit Rate
- **Type**: Gauge (0-100%)
- **Metric**: `100 * rate(katana_db_operation_get_hits[5m]) / (rate(katana_db_operation_get_hits[5m]) + rate(katana_db_operation_get_misses[5m]))`
- **Purpose**: Measure the efficiency of get operations
- **What to look for**:
  - Higher percentages (>90%) indicate good cache locality
  - Lower percentages may indicate poor data access patterns
  - Sudden drops may indicate cache invalidation or new data access patterns

#### Operation Totals (Stat Panels)
- **Total Gets**: Cumulative get operations (hits + misses)
- **Get Hits**: Number of successful get operations
- **Get Misses**: Number of get operations that didn't find a value
- **Total Puts**: Cumulative put operations
- **Total Deletes**: Cumulative delete operations
- **Delete Success**: Number of successful delete operations
- **Delete Failures**: Number of failed delete operations

### 3. Database Performance

All performance panels show **percentile latencies** (p99, p95, p50) to understand the distribution of operation times.

#### Transaction Commit Time (p99)
- **Type**: Time series chart
- **Metrics**: Histogram quantiles of `katana_db_transaction_commit_time_seconds`
- **Purpose**: Monitor transaction commit latency
- **What to look for**:
  - p99 should remain low and stable
  - Increasing latencies indicate database contention or I/O issues
  - Large gap between p50 and p99 indicates inconsistent performance

#### Get Operation Time (p99)
- **Type**: Time series chart
- **Metrics**: Histogram quantiles of `katana_db_operation_get_time_seconds`
- **Purpose**: Monitor read operation latency
- **What to look for**:
  - Get operations should be fast (microseconds to low milliseconds)
  - Increasing latencies may indicate disk I/O bottlenecks

#### Put Operation Time (p99)
- **Type**: Time series chart
- **Metrics**: Histogram quantiles of `katana_db_operation_put_time_seconds`
- **Purpose**: Monitor write operation latency
- **What to look for**:
  - Put operations are typically slower than gets
  - Sudden spikes may indicate fsync or flush operations

#### Delete Operation Time (p99)
- **Type**: Time series chart
- **Metrics**: Histogram quantiles of `katana_db_operation_delete_time_seconds`
- **Purpose**: Monitor delete operation latency
- **What to look for**:
  - Should be similar to put operation times
  - Consistently high latencies may indicate fragmentation

## Alerting Recommendations

Consider setting up alerts for:

1. **High Transaction Failure Rate**
   - Alert: `rate(katana_db_transaction_commits_failed[5m]) > 0.1`
   - Severity: Critical

2. **Low Cache Hit Rate**
   - Alert: `100 * rate(katana_db_operation_get_hits[5m]) / (rate(katana_db_operation_get_hits[5m]) + rate(katana_db_operation_get_misses[5m])) < 70`
   - Severity: Warning

3. **High p99 Commit Latency**
   - Alert: `histogram_quantile(0.99, rate(katana_db_transaction_commit_time_seconds_bucket[5m])) > 1.0`
   - Severity: Warning

4. **High Operation Latency**
   - Alert: `histogram_quantile(0.99, rate(katana_db_operation_get_time_seconds_bucket[5m])) > 0.1`
   - Severity: Warning

## Troubleshooting

### High Transaction Failure Rate
- Check disk space and I/O capacity
- Review application logs for transaction conflicts
- Consider database tuning parameters

### Low Cache Hit Rate
- Review data access patterns in application code
- Consider if working set exceeds available memory
- Check if queries are properly optimized

### High Operation Latency
- Check system I/O wait times
- Review database size and fragmentation
- Consider if database needs compaction
- Check for concurrent operations causing contention

### Growing Delete Failures
- May be expected if deleting non-existent keys
- Check application logic for proper key management
- Review error logs for unexpected failures

## Related Metrics

The dashboard also includes pre-existing panels for:
- **Database Table Sizes**: Individual table sizes over time
- **Total Size**: Aggregate database size
- **Freelist Size**: Available space for reuse
- **Table Entries**: Number of entries per table
- **Table Pages**: Page count per table

These provide complementary storage-level insights to the operational metrics.

## Accessing the Dashboard

1. Ensure Katana is running with metrics enabled:
   ```bash
   katana --metrics --metrics.addr 0.0.0.0 --metrics.port 9100
   ```

2. Start the monitoring stack:
   ```bash
   cd monitoring
   docker-compose up -d
   ```

3. Access Grafana at http://localhost:3000
   - Username: `admin`
   - Password: `admin`

4. Navigate to the "Katana Overview" dashboard

## Metrics Prefix Reference

All database metrics use the following prefixes:
- `katana_db_transaction_*` - Transaction lifecycle metrics
- `katana_db_operation_*` - CRUD operation metrics
- `katana_db_table_*` - Table-level statistics (gauges)
- `katana_db_freelist` - Database freelist size

For detailed implementation, see `crates/storage/db/src/mdbx/metrics.rs`.