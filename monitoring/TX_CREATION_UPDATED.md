# Transaction Creation Panel - Updated

## ✅ Changed: Shows When Transactions Are Created (Not Rate)

The panel has been updated to show **when transactions are being created** rather than the rate per second.

## What Changed

### Before (Rate-based)
- Showed transactions **per second**
- Used `rate()` function
- Unit: requests per second (reqps)
- Stacked area chart

### After (Event-based) ✨
- Shows **number of new transactions** in each time bucket
- Uses `increase()` function with automatic interval
- Unit: transaction count (discrete numbers)
- Line chart (not stacked)
- **You can see exactly when transactions are created!**

## How It Works

The panel now uses `increase(katana_db_transaction_ro_created[$__rate_interval])`:
- `increase()` shows the actual count increase over each time bucket
- `$__rate_interval` automatically adjusts based on graph resolution
- You see discrete spikes when transactions are created
- Zero or flat lines when no transactions are happening

## Visualization

```
Transaction Creation
─────────────────────────────────────────────────────

        ╱╲                    ╱╲
       ╱  ╲                  ╱  ╲      ← RO Transactions (blue)
   ───╱    ╲────────────────╱    ╲────
   
      ╱╲        ╱╲              ╱╲     ← RW Transactions (orange)
  ───╱  ╲──────╱  ╲────────────╱  ╲───
  
        ╱╲                    ╱╲
       ╱  ╲                  ╱  ╲      ← Total (purple dashed)
   ───╱    ╲────────────────╱    ╲────

─────────────────────────────────────────────────────
  10:00    10:05    10:10    10:15    10:20

Legend: | Series | Last | Mean | Max |
        |--------|------|------|-----|
        | RO     | 25   | 18   | 42  | ← 25 new RO txs in last bucket
        | RW     | 3    | 2    | 5   | ← 3 new RW txs in last bucket
        | Total  | 28   | 20   | 47  | ← 28 total new txs
```

## Reading the Panel

### Active System
When transactions are being created, you'll see:
- **Spikes** when new transactions occur
- **Height of spike** = number of transactions created in that time window
- **Flat lines** or zeros = no transaction activity

### Example Values
- **Last: 15** = 15 new transactions were created in the most recent time bucket
- **Mean: 12** = Average of 12 new transactions per time bucket
- **Max: 50** = Highest spike was 50 transactions in one time bucket

### Time Resolution
The time bucket size adjusts automatically:
- **5m view**: Buckets might be 5-10 seconds each
- **1h view**: Buckets might be 30 seconds each
- **24h view**: Buckets might be 2-5 minutes each

## Use Cases

### 1. Detect Transaction Activity
```
No activity:  ──────────────────────────
Activity:     ─╱╲──╱╲────╱╲╱╲╱╲────╱╲─
```
Instantly see when your system is processing transactions.

### 2. Identify Batch Operations
```
Batch:        ───╱█╲───────╱█╲────────
              (big spike) (big spike)
```
Large spikes indicate batch transaction creation.

### 3. Monitor Steady Load
```
Steady:       ─╱╲╱╲╱╲╱╲╱╲╱╲╱╲╱╲╱╲╱╲─
```
Consistent pattern shows steady transaction flow.

### 4. Spot Anomalies
```
Normal:       ─╱╲╱╲╱╲──────╱╲╱╲╱╲───
Spike:        ────────╱████╲─────────
              (unexpected burst)
```
Unusual spikes may indicate issues or attacks.

## Query Details

### Read-Only Transactions
```promql
increase(katana_db_transaction_ro_created[$__rate_interval])
```

### Read-Write Transactions
```promql
increase(katana_db_transaction_rw_created[$__rate_interval])
```

### Total Transactions
```promql
increase(katana_db_transaction_ro_created[$__rate_interval]) + 
increase(katana_db_transaction_rw_created[$__rate_interval])
```

## Compared to Rate

### This Panel (Increase)
- ✅ Shows WHEN transactions are created
- ✅ Discrete counts (5, 10, 15 transactions)
- ✅ Easy to see idle periods (zero values)
- ✅ Spikes are actual transaction bursts
- Better for: Event detection, activity monitoring

### Rate Panel (Still Available)
- Shows transactions PER SECOND
- Continuous values (1.5, 2.3, 5.7 tx/s)
- Smoothed averages
- Better for: Capacity planning, throughput analysis

## Access

🌐 **Grafana**: http://localhost:3000

Navigate to **Katana Overview** → **Database Transactions** section → Top panel

## Regenerate

To update or customize:
```bash
cd monitoring
python3 add_tx_creation_panel.py
docker-compose restart grafana
```

---

**Updated**: 2024-11-24  
**Query Type**: `increase()` instead of `rate()`  
**Unit**: Transaction count (not per second)  
**Stacking**: Disabled (individual lines)
