# Grafana Dashboard Visual Guide

This guide provides a visual representation of the Katana Overview dashboard layout and panel descriptions.

## Dashboard Layout

```
┌─────────────────────────────────────────────────────────────────────────┐
│                          KATANA OVERVIEW DASHBOARD                       │
└─────────────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────────────┐
│ ▼ DATABASE STORAGE                                                       │
├──────────────────────────────────┬──────────────────────────────────────┤
│  📈 Table Sizes Over Time        │  📊 Total Database Size (Stat)       │
│  (Line chart showing growth of   │  (Single value with trend)           │
│   individual database tables)    │                                      │
│                                  │                                      │
│  • BlockBodyIndices              │  💾 2.1 GB                           │
│  • Headers                       │     ↗ +15% (trending up)             │
│  • Transactions                  │                                      │
│  • Receipts                      │                                      │
│  • ContractStorage               │                                      │
│  • ... (30+ tables tracked)      │                                      │
├──────────────────────────────────┴──────────────────────────────────────┤
│  📈 Freelist Size Over Time                                             │
│  (Shows available space for reuse in the database)                      │
│                                                                          │
│  Indicates database fragmentation and cleanup efficiency                │
└─────────────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────────────┐
│ ▼ DATABASE TRANSACTIONS                                                  │
├──────────────────────────────────┬──────────────────────────────────────┤
│  📈 Transaction Creation Rate     │  📈 Transaction Commit Status        │
│                                  │                                      │
│  ─── Read-Only (RO)              │  ─── Successful                      │
│  ─── Read-Write (RW)             │  ─── Failed                          │
│                                  │  ─── Aborted                         │
│  Shows tx/sec for each type      │  Shows commit outcomes over time     │
├────────┬────────┬────────┬───────┴──────────────────────────────────────┤
│ Total  │ Total  │Success │ Failed/Aborted                               │
│   RO   │   RW   │Commits │   Commits                                    │
│        │        │        │                                              │
│ 22,732 │  642   │  427   │    215                                       │
└────────┴────────┴────────┴──────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────────────┐
│ ▼ DATABASE OPERATIONS                                                    │
├──────────────────────────────────┬──────────────────────────────────────┤
│  📈 Operation Rate by Type        │  📈 Delete Success Rate              │
│                                  │                                      │
│  ─── Puts                        │  ─── Successful                      │
│  ─── Gets (Total)                │  ─── Failed                          │
│  ─── Deletes (Total)             │                                      │
│  ─── Clears                      │  Tracks delete operation outcomes    │
│                                  │                                      │
│  Shows ops/sec for each type     │                                      │
├──────────────┬────────┬──────────┼──────────────┬───────────────────────┤
│   🎯 Cache   │ Total  │   Gets   │ Total  Puts  │  Total Deletes        │
│   Hit Rate   │  Gets  │          │              │                       │
│              │        │          │              │                       │
│    ┌───┐    │ 14.4M  │ Hits:    │   10.2M      │      99.6K            │
│    │███│    │        │ 14.4M    │              │                       │
│    │███│98% │        │          │              │                       │
│    │███│    │        │ Misses:  │              │                       │
│    └───┘    │        │  902K    │              │                       │
│             │        │          │              │                       │
│  Green      │        ├─────┬────┤              ├──────────┬────────────┤
│  indicator  │        │ Hit │Miss│              │ Success  │  Failures  │
│             │        │14.4M│902K│              │  10,798  │   88,773   │
└─────────────┴────────┴─────┴────┴──────────────┴──────────┴────────────┘

┌─────────────────────────────────────────────────────────────────────────┐
│ ▼ DATABASE PERFORMANCE                                                   │
├──────────────────────────────────┬──────────────────────────────────────┤
│  📈 Transaction Commit Time       │  📈 Get Operation Time               │
│      (Latency Percentiles)       │      (Latency Percentiles)           │
│                                  │                                      │
│  ─── p99 (99th percentile)       │  ─── p99                             │
│  ─── p95 (95th percentile)       │  ─── p95                             │
│  ─── p50 (median)                │  ─── p50                             │
│                                  │                                      │
│  Shows commit duration in sec    │  Shows read latency in sec           │
├──────────────────────────────────┼──────────────────────────────────────┤
│  📈 Put Operation Time            │  📈 Delete Operation Time            │
│      (Latency Percentiles)       │      (Latency Percentiles)           │
│                                  │                                      │
│  ─── p99                         │  ─── p99                             │
│  ─── p95                         │  ─── p95                             │
│  ─── p50                         │  ─── p50                             │
│                                  │                                      │
│  Shows write latency in sec      │  Shows delete latency in sec         │
└──────────────────────────────────┴──────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────────────┐
│ ▼ EXECUTION                                                              │
├──────────────────────────────────┬──────────────────────────────────────┤
│  📈 L1 Gas Consumption            │  📈 Transaction Execution Stats      │
│  (Total gas processed over time) │                                      │
└──────────────────────────────────┴──────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────────────┐
│ ▼ RPC                                                                    │
├──────────────────────────────────┬──────────────────────────────────────┤
│  🔥 Request Rate Heatmap          │  📈 RPC Call Success/Failure         │
│  (Color-coded request frequency  │  (by method)                         │
│   by method and time)            │                                      │
├──────────────────────────────────┴──────────────────────────────────────┤
│  📈 Response Time Distribution                                          │
│  (Latency heatmap showing p-values for each RPC method)                 │
└─────────────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────────────┐
│ ▼ MEMORY                                                                 │
├──────────────────────────────────┬──────────────────────────────────────┤
│  📈 jemalloc Memory Usage         │  📊 Memory Breakdown (Pie Chart)     │
│  • Allocated                     │                                      │
│  • Active                        │  🟦 Allocated (850 MB)               │
│  • Mapped                        │  🟩 Active (1.2 GB)                  │
│  • Metadata                      │  🟨 Metadata (108 MB)                │
└──────────────────────────────────┴──────────────────────────────────────┘
```

## Panel Descriptions

### Database Storage Section

#### 📈 Table Sizes Over Time
- **Type**: Time series (multi-line)
- **Shows**: Size in bytes of each database table
- **Use**: Track storage growth patterns, identify large tables
- **Tables tracked**: BlockBodyIndices, Headers, Transactions, Receipts, ContractStorage, Classes, etc.

#### 📊 Total Database Size
- **Type**: Stat panel
- **Shows**: Aggregate size of all tables
- **Use**: Quick view of total storage consumption
- **Includes**: Trend indicator showing growth rate

#### 📈 Freelist Size
- **Type**: Time series
- **Shows**: Size of database freelist (reusable space)
- **Use**: Monitor database fragmentation and space efficiency
- **Note**: Large freelist may indicate need for compaction

### Database Transactions Section

#### 📈 Transaction Creation Rate
- **Type**: Time series (2 lines)
- **Metrics**: Read-Only and Read-Write transactions per second
- **Use**: Understand workload distribution between reads and writes
- **Typical pattern**: RO >> RW for most blockchain operations

#### 📈 Transaction Commit Status
- **Type**: Time series (3 lines)
- **Metrics**: Successful commits, failed commits, and aborts per second
- **Use**: Monitor transaction health and identify commit issues
- **Alert on**: Increasing failure or abort rates

#### Statistics Panels (4 panels)
- **Total RO Transactions**: Cumulative read-only transaction count
- **Total RW Transactions**: Cumulative read-write transaction count
- **Successful Commits**: Total commits that completed successfully
- **Failed/Aborted**: Sum of failed commits and aborted transactions

### Database Operations Section

#### 📈 Operation Rate by Type
- **Type**: Time series (4 lines)
- **Metrics**: Rate of Put, Get, Delete, and Clear operations
- **Use**: Understand CRUD operation distribution
- **Typical pattern**: Gets > Puts > Deletes >> Clears

#### 📈 Delete Success Rate
- **Type**: Time series (2 lines)
- **Metrics**: Successful vs failed delete operations per second
- **Use**: Track delete reliability
- **Note**: Failed deletes may be expected (key not found)

#### 🎯 Cache Hit Rate (Gauge)
- **Type**: Gauge (0-100%)
- **Formula**: `(Get Hits / Total Gets) * 100`
- **Use**: **Key performance indicator** for cache efficiency
- **Color coding**:
  - 🟢 Green (90-100%): Excellent cache performance
  - 🟡 Yellow (70-90%): Acceptable, room for optimization
  - 🔴 Red (<70%): Poor cache efficiency, investigate access patterns
- **Action items**:
  - Low hit rate → Review query patterns
  - Sudden drop → Check for workload changes or cache invalidation

#### Operation Statistics (8 panels)
- **Total Gets**: Sum of hits and misses
- **Get Hits**: Successful get operations (value found)
- **Get Misses**: Get operations where value was not found
- **Total Puts**: All put/write operations
- **Total Deletes**: All delete operations attempted
- **Delete Success**: Successful delete operations
- **Delete Failures**: Failed delete operations

### Database Performance Section

All performance panels show **percentile distributions** to understand latency characteristics:

- **p99**: 99% of operations complete within this time (worst case for 99%)
- **p95**: 95% of operations complete within this time
- **p50**: Median operation time (typical case)

#### 📈 Transaction Commit Time
- **Shows**: Time to commit transactions (in seconds)
- **Use**: Identify slow commits and transaction bottlenecks
- **Typical values**: <1ms for p50, <10ms for p99
- **Alert on**: p99 > 1 second

#### 📈 Get Operation Time
- **Shows**: Read operation latency (in seconds)
- **Use**: Monitor query performance
- **Typical values**: <100μs for p50, <1ms for p99
- **Alert on**: p99 > 100ms

#### 📈 Put Operation Time
- **Shows**: Write operation latency (in seconds)
- **Use**: Track write performance
- **Typical values**: <1ms for p50, <10ms for p99
- **Alert on**: p99 > 100ms

#### 📈 Delete Operation Time
- **Shows**: Delete operation latency (in seconds)
- **Use**: Monitor cleanup operation performance
- **Typical values**: Similar to puts
- **Alert on**: p99 > 100ms

## Using the Dashboard

### For Daily Monitoring

1. **Check Cache Hit Rate** (gauge in Operations section)
   - Should be >90% for healthy operation
   - Drop below 70% requires investigation

2. **Review Transaction Commit Status**
   - Failures/aborts should be rare or zero
   - Consistent failures indicate issues

3. **Monitor Operation Rates**
   - Understand your workload patterns
   - Spot unusual spikes or drops

4. **Check Performance Percentiles**
   - p99 should remain stable
   - Sudden increases indicate bottlenecks

### For Performance Tuning

1. **Analyze Cache Efficiency**
   - Use Get Hits vs Misses to understand cache behavior
   - Low hit rate → optimize data access patterns

2. **Review Latency Distribution**
   - Large gap between p50 and p99 → inconsistent performance
   - Investigate outliers causing high p99

3. **Monitor Write Performance**
   - High put latency → check I/O subsystem
   - Consider batching strategies

### For Troubleshooting

1. **Transaction Failures**
   - Check commit status chart for spikes
   - Review system logs for error details
   - Check disk space and I/O

2. **Performance Degradation**
   - Compare current vs historical p99 values
   - Check if operation rates have increased
   - Review memory and CPU usage

3. **Cache Issues**
   - Sudden drop in hit rate → workload change
   - Consistently low hit rate → access pattern problem
   - Check application query logic

## Time Range Selection

Use the time picker (top right) to focus on specific periods:

- **Last 5 minutes**: Real-time monitoring
- **Last 1 hour**: Recent performance review
- **Last 6 hours**: Troubleshooting issues
- **Last 24 hours**: Daily patterns and trends
- **Last 7 days**: Weekly patterns and capacity planning

## Refresh Rate

Configure auto-refresh (top right):

- **5s**: Real-time monitoring during incidents
- **30s**: Active monitoring
- **1m**: General dashboard viewing
- **5m**: Background monitoring

## Tips

- **Hover over graphs** to see exact values at specific times
- **Click and drag** on graphs to zoom into time ranges
- **Click legend items** to toggle series on/off
- **Use shift+click** on legend to isolate a single series
- **Right-click panels** to access more options (view query, export, etc.)

## Related Documentation

- **Detailed Metrics Guide**: See `DATABASE_MONITORING.md`
- **Setup Instructions**: See `README.md`
- **Panel Regeneration**: See `add_db_panels.py`
