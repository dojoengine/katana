# Database Metrics Quick Reference

A quick reference card for the most important Katana database metrics.

## 🔥 Top 5 Metrics to Watch

### 1. Cache Hit Rate
```promql
100 * rate(katana_db_operation_get_hits[5m]) / 
(rate(katana_db_operation_get_hits[5m]) + rate(katana_db_operation_get_misses[5m]))
```
- **Target**: >90% (green), 70-90% (yellow), <70% (red)
- **What it means**: Percentage of get operations that found cached data
- **When to alert**: Drops below 70%
- **Action**: Review data access patterns, check if working set exceeds memory

### 2. Transaction Commit Failure Rate
```promql
rate(katana_db_transaction_commits_failed[5m]) + 
rate(katana_db_transaction_aborts[5m])
```
- **Target**: Near zero
- **What it means**: Failed or aborted transactions per second
- **When to alert**: >1% of total commits fail
- **Action**: Check disk space, I/O capacity, and application logs

### 3. p99 Transaction Commit Time
```promql
histogram_quantile(0.99, rate(katana_db_transaction_commit_time_seconds_bucket[5m]))
```
- **Target**: <100ms (good), <1s (acceptable)
- **What it means**: 99% of commits complete within this time
- **When to alert**: >1 second
- **Action**: Check I/O wait times, review transaction size, consider batching

### 4. p99 Get Operation Time
```promql
histogram_quantile(0.99, rate(katana_db_operation_get_time_seconds_bucket[5m]))
```
- **Target**: <1ms (excellent), <100ms (acceptable)
- **What it means**: 99% of read operations complete within this time
- **When to alert**: >100ms
- **Action**: Check disk I/O, review query complexity, verify indexes

### 5. Database Growth Rate
```promql
rate(sum(katana_db_table_size)[1h])
```
- **Target**: Steady, predictable growth
- **What it means**: Rate of database size increase
- **When to alert**: Unexpected spikes or sustained high growth
- **Action**: Review data retention policies, check for data bloat

## 📊 Metric Categories

### Transaction Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `katana_db_transaction_ro_created` | Counter | Read-only transactions created |
| `katana_db_transaction_rw_created` | Counter | Read-write transactions created |
| `katana_db_transaction_commits_successful` | Counter | Successful commits |
| `katana_db_transaction_commits_failed` | Counter | Failed commits |
| `katana_db_transaction_aborts` | Counter | Aborted transactions |
| `katana_db_transaction_commit_time_seconds` | Histogram | Commit duration |

### Operation Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `katana_db_operation_get_hits` | Counter | Get operations that found value |
| `katana_db_operation_get_misses` | Counter | Get operations with no value found |
| `katana_db_operation_get_time_seconds` | Histogram | Get operation duration |
| `katana_db_operation_puts` | Counter | Put operations |
| `katana_db_operation_put_time_seconds` | Histogram | Put operation duration |
| `katana_db_operation_deletes_successful` | Counter | Successful delete operations |
| `katana_db_operation_deletes_failed` | Counter | Failed delete operations |
| `katana_db_operation_delete_time_seconds` | Histogram | Delete operation duration |
| `katana_db_operation_clears` | Counter | Clear operations |
| `katana_db_operation_clear_time_seconds` | Histogram | Clear operation duration |

### Storage Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `katana_db_table_size{table="..."}` | Gauge | Table size in bytes |
| `katana_db_table_entries{table="..."}` | Gauge | Number of entries in table |
| `katana_db_table_pages{table="...",type="..."}` | Gauge | Page count per table |
| `katana_db_freelist` | Gauge | Freelist size (reusable space) |

## 🎯 Common Queries

### Operation Rate by Type
```promql
# Put operations per second
rate(katana_db_operation_puts[5m])

# Get operations per second (total)
rate(katana_db_operation_get_hits[5m]) + rate(katana_db_operation_get_misses[5m])

# Delete operations per second (total)
rate(katana_db_operation_deletes_successful[5m]) + rate(katana_db_operation_deletes_failed[5m])
```

### Success Rates
```promql
# Delete success rate (%)
100 * rate(katana_db_operation_deletes_successful[5m]) /
(rate(katana_db_operation_deletes_successful[5m]) + rate(katana_db_operation_deletes_failed[5m]))

# Commit success rate (%)
100 * rate(katana_db_transaction_commits_successful[5m]) /
(rate(katana_db_transaction_commits_successful[5m]) + rate(katana_db_transaction_commits_failed[5m]))
```

### Latency Percentiles
```promql
# p50 (median)
histogram_quantile(0.50, rate(katana_db_operation_get_time_seconds_bucket[5m]))

# p95 (95th percentile)
histogram_quantile(0.95, rate(katana_db_operation_get_time_seconds_bucket[5m]))

# p99 (99th percentile)
histogram_quantile(0.99, rate(katana_db_operation_get_time_seconds_bucket[5m]))
```

### Storage Analysis
```promql
# Total database size
sum(katana_db_table_size)

# Top 5 largest tables
topk(5, katana_db_table_size)

# Freelist utilization (%)
100 * katana_db_freelist / sum(katana_db_table_size)
```

## 🚨 Alert Thresholds

### Critical Alerts

```promql
# High transaction failure rate
rate(katana_db_transaction_commits_failed[5m]) > 0.1

# Very high commit latency
histogram_quantile(0.99, rate(katana_db_transaction_commit_time_seconds_bucket[5m])) > 5.0

# No successful operations for 1 minute
rate(katana_db_operation_puts[1m]) == 0
```

### Warning Alerts

```promql
# Low cache hit rate
100 * rate(katana_db_operation_get_hits[5m]) / 
(rate(katana_db_operation_get_hits[5m]) + rate(katana_db_operation_get_misses[5m])) < 70

# High p99 commit latency
histogram_quantile(0.99, rate(katana_db_transaction_commit_time_seconds_bucket[5m])) > 1.0

# High p99 get latency
histogram_quantile(0.99, rate(katana_db_operation_get_time_seconds_bucket[5m])) > 0.1

# Growing delete failures
rate(katana_db_operation_deletes_failed[5m]) > 10
```

## 📈 Performance Baselines

### Expected Values (Typical Workload)

| Metric | Good | Acceptable | Poor |
|--------|------|------------|------|
| Cache hit rate | >95% | 70-95% | <70% |
| p99 commit time | <10ms | <100ms | >1s |
| p99 get time | <1ms | <10ms | >100ms |
| p99 put time | <10ms | <100ms | >1s |
| Commit success rate | >99.9% | >99% | <99% |
| Transaction abort rate | <0.1% | <1% | >1% |

### Workload Patterns

**Read-Heavy** (typical for blockchain queries):
- RO transactions >> RW transactions (10:1 or higher)
- Get operations >> Put operations
- High cache hit rate (>90%)

**Write-Heavy** (during sync or heavy transaction processing):
- More balanced RO/RW ratio
- Put operations increase significantly
- Cache hit rate may temporarily decrease

**Maintenance** (compaction, cleanup):
- High delete operation rate
- Increased clear operations
- Temporary performance degradation acceptable

## 🔍 Troubleshooting Checklist

### Low Cache Hit Rate
- [ ] Check working set size vs available memory
- [ ] Review query patterns for sequential access
- [ ] Verify data locality in application logic
- [ ] Consider if recently accessed data is being reused
- [ ] Check for cache-busting operations (mass updates)

### High Latency
- [ ] Check system I/O wait times (`iostat`, `iotop`)
- [ ] Verify disk performance (IOPS, throughput)
- [ ] Review transaction sizes (large commits = higher latency)
- [ ] Check for concurrent operations causing contention
- [ ] Monitor CPU usage (high CPU = processing bottleneck)

### Transaction Failures
- [ ] Check disk space (`df -h`)
- [ ] Review error logs for specific failure reasons
- [ ] Verify database permissions
- [ ] Check for file system issues
- [ ] Monitor for database corruption

### Growing Database Size
- [ ] Review table sizes to identify growth sources
- [ ] Check freelist size (high = fragmentation)
- [ ] Consider database compaction
- [ ] Review data retention policies
- [ ] Verify cleanup operations are running

## 📝 Quick Commands

### Check Metrics Endpoint
```bash
curl http://localhost:9100 | grep katana_db
```

### Query Prometheus
```bash
# Get current value
curl -s 'http://localhost:9090/api/v1/query?query=katana_db_transaction_ro_created'

# Get range
curl -s 'http://localhost:9090/api/v1/query_range?query=rate(katana_db_operation_puts[5m])&start=2024-01-01T00:00:00Z&end=2024-01-01T01:00:00Z&step=15s'
```

### Grafana Dashboard
```bash
# Access dashboard
open http://localhost:3000

# Restart Grafana
docker-compose restart grafana
```

## 📚 Related Resources

- **Full Monitoring Guide**: `DATABASE_MONITORING.md`
- **Dashboard Guide**: `DASHBOARD_GUIDE.md`
- **Setup Instructions**: `README.md`
- **Metrics Implementation**: `crates/storage/db/src/mdbx/metrics.rs`

---

**Last Updated**: 2024-11  
**Version**: 1.0  
**Katana Version**: v1.0.12+