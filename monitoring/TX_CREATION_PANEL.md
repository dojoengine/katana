# Transaction Creation Rate Panel

## Overview

A prominent full-width panel has been added to visualize transaction creation rates in real-time.

## Panel Details

### Location
- **Section**: Database Transactions (top panel)
- **Position**: Full width (24 columns), 8 rows tall
- **ID**: 300

### Visualization

The panel shows a **stacked area chart** with three series:

1. **Read-Only (RO)** - Blue fill
   - Metric: `rate(katana_db_transaction_ro_created[1m])`
   - Tracks read-only transaction creation rate
   
2. **Read-Write (RW)** - Orange fill
   - Metric: `rate(katana_db_transaction_rw_created[1m])`
   - Tracks read-write transaction creation rate
   
3. **Total** - Purple dashed line (no fill)
   - Metric: `rate(katana_db_transaction_ro_created[1m]) + rate(katana_db_transaction_rw_created[1m])`
   - Shows total transaction rate across both types

### Features

✨ **Stacked visualization**: Easy to see the proportion of RO vs RW transactions
📊 **Legend with statistics**: Shows Last, Mean, and Max values for each series
⚡ **1-minute rate window**: More responsive to changes than 5-minute window
🎨 **Color-coded**: Blue (RO), Orange (RW), Purple (Total)
📈 **Smooth interpolation**: Clean, easy-to-read lines

### Use Cases

#### Monitor Workload Balance
- See the ratio of read vs write operations
- Typical blockchain: RO >> RW (10:1 or higher)
- Heavy sync: More balanced ratio

#### Identify Load Spikes
- Sudden increases in transaction creation
- Correlate with application events
- Plan for capacity

#### Performance Analysis
- Compare transaction rates to commit rates
- Identify transaction queuing issues
- Track throughput over time

### Example Interpretations

**Healthy Pattern**:
```
RO: ████████████ 150 tx/s
RW: ██           15 tx/s
Total: ▬▬▬▬▬▬▬▬▬ 165 tx/s
```
- High RO rate indicates active querying
- Low RW rate is normal for read-heavy workload

**Sync Pattern**:
```
RO: █████ 50 tx/s
RW: ████  40 tx/s
Total: ▬▬▬▬ 90 tx/s
```
- More balanced during blockchain sync
- Higher write activity expected

**Idle Pattern**:
```
RO: ▂ 5 tx/s
RW: ▁ 1 tx/s
Total: ▬ 6 tx/s
```
- Low activity, system idle
- Background maintenance only

## Accessing the Panel

1. Open Grafana: http://localhost:3000
2. Navigate to "Katana Overview" dashboard
3. Expand "Database Transactions" section
4. The transaction creation panel is at the top (full width)

## Customization

### Change Time Window
Edit the `[1m]` parameter in queries to adjust responsiveness:
- `[30s]` - More responsive, noisier
- `[1m]` - Good balance (current)
- `[5m]` - Smoother, less responsive

### Remove Stacking
In panel edit mode:
1. Go to "Field" tab
2. Find "Stack series"
3. Change from "Normal" to "None"

### Adjust Colors
In the panel JSON or UI:
- RO: Change "fixedColor" from "blue"
- RW: Change "fixedColor" from "orange"
- Total: Change "fixedColor" from "purple"

## Script Usage

To regenerate or update this panel:

```bash
cd monitoring
python3 add_tx_creation_panel.py
docker-compose restart grafana
```

The script:
- Creates the panel if it doesn't exist
- Updates it if it already exists (ID 300)
- Automatically handles positioning

## Related Metrics

This panel complements other transaction metrics:
- **Transaction Commit Status**: Shows what happens after creation
- **Transaction Statistics**: Cumulative counts
- **Commit Time**: Performance of transaction completion

## Alert Recommendations

Consider alerts on transaction creation anomalies:

```promql
# Unusually high transaction rate (adjust threshold for your workload)
rate(katana_db_transaction_ro_created[1m]) + rate(katana_db_transaction_rw_created[1m]) > 1000

# No transactions for extended period (system issue)
rate(katana_db_transaction_ro_created[5m]) + rate(katana_db_transaction_rw_created[5m]) == 0

# Unusual RW to RO ratio (>50% writes)
rate(katana_db_transaction_rw_created[1m]) / (rate(katana_db_transaction_ro_created[1m]) + rate(katana_db_transaction_rw_created[1m])) > 0.5
```

## Troubleshooting

### No Data Showing
- Verify Katana is running and creating transactions
- Check Prometheus is scraping: http://localhost:9090/targets
- Verify metrics exist: `curl http://localhost:9100 | grep transaction_ro_created`

### Flat Line at Zero
- System may be idle (expected)
- Check if Katana is processing blocks
- Verify time range in Grafana covers active period

### Erratic Spikes
- Normal with 1-minute window during variable load
- Consider using 5-minute window for smoother view
- May indicate actual workload variability

