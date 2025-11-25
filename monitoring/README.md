# Katana Monitoring

This directory contains a complete monitoring setup for Katana using Prometheus and Grafana, with comprehensive database performance tracking.

## Overview

The monitoring stack provides:
- **Prometheus**: Metrics collection and storage
- **Grafana**: Visualization dashboards with pre-configured panels
- **Database Metrics**: Comprehensive MDBX database monitoring

## Quick Start

### Option 1: Run Everything in Docker

```bash
docker-compose up -d
```

This starts Katana, Prometheus, and Grafana all in Docker containers.

### Option 2: Run Katana Locally (Recommended for Development)

1. Start Katana locally with metrics enabled:
   ```bash
   katana --metrics --metrics.addr 0.0.0.0 --metrics.port 9100
   ```

2. Start only the monitoring services:
   ```bash
   # Comment out the katana service in docker-compose.yml first
   docker-compose up -d
   ```

## Accessing Services

- **Grafana Dashboard**: http://localhost:3000
  - Username: `admin`
  - Password: `admin`
  - The Overview dashboard will load automatically

- **Prometheus UI**: http://localhost:9090
  - Check targets at http://localhost:9090/targets

- **Katana Metrics**: http://localhost:9100
  - Raw metrics endpoint (when running locally)

## Dashboard Sections

The Grafana Overview dashboard includes:

### 1. Database Storage
- Table sizes over time
- Total database size
- Freelist size
- Table entries and pages

### 2. Database Transactions
- Transaction creation rate (RO vs RW)
- Commit success/failure rates
- Transaction abort tracking
- Cumulative transaction statistics

### 3. Database Operations
- CRUD operation rates (Get, Put, Delete, Clear)
- Delete success vs failure rates
- **Cache hit rate gauge** - Shows percentage of successful get operations
- Operation totals and breakdowns

### 4. Database Performance
- Transaction commit time (p99, p95, p50 percentiles)
- Get operation latency
- Put operation latency
- Delete operation latency

### 5. Execution Metrics
- L1 gas consumption
- Transaction execution stats

### 6. RPC Metrics
- Request rates by method
- Success/failure tracking
- Response time heatmaps

### 7. Memory Metrics
- jemalloc memory statistics
- Allocation tracking

## Configuration

### Prometheus Configuration

Edit `prometheus/config.yml` to adjust:
- Scrape intervals (default: 5s)
- Target endpoints
- Retention policies

Current configuration:
```yaml
scrape_configs:
  - job_name: katana
    metrics_path: "/"
    scrape_interval: 5s
    static_configs:
      - targets: ["host.docker.internal:9100"]  # For local Katana
```

### Grafana Configuration

Dashboards are located in `grafana/dashboards/`:
- `overview.json` - Main dashboard with all panels

Data sources are auto-provisioned from `grafana/datasources/`.

## Customizing Database Panels

To regenerate or customize the database monitoring panels:

```bash
cd monitoring
python3 add_db_panels.py
```

This script automatically adds comprehensive database monitoring panels to the dashboard.

## Available Metrics

### Database Transaction Metrics
- `katana_db_transaction_ro_created` - Read-only transactions
- `katana_db_transaction_rw_created` - Read-write transactions
- `katana_db_transaction_commits_successful` - Successful commits
- `katana_db_transaction_commits_failed` - Failed commits
- `katana_db_transaction_aborts` - Aborted transactions
- `katana_db_transaction_commit_time_seconds` - Commit duration histogram

### Database Operation Metrics
- `katana_db_operation_get_hits` - Successful get operations
- `katana_db_operation_get_misses` - Get operations that found no value
- `katana_db_operation_get_time_seconds` - Get duration histogram
- `katana_db_operation_puts` - Put operations
- `katana_db_operation_put_time_seconds` - Put duration histogram
- `katana_db_operation_deletes_successful` - Successful deletes
- `katana_db_operation_deletes_failed` - Failed deletes
- `katana_db_operation_delete_time_seconds` - Delete duration histogram
- `katana_db_operation_clears` - Clear operations
- `katana_db_operation_clear_time_seconds` - Clear duration histogram

### Database Storage Metrics
- `katana_db_table_size{table="..."}` - Size of each table in bytes
- `katana_db_table_entries{table="..."}` - Number of entries per table
- `katana_db_table_pages{table="...",type="..."}` - Page count per table
- `katana_db_freelist` - Database freelist size

For detailed monitoring information, see [DATABASE_MONITORING.md](./DATABASE_MONITORING.md).

## Troubleshooting

### Prometheus Can't Connect to Katana

1. **Check Katana is running with metrics enabled**:
   ```bash
   curl http://localhost:9100
   ```
   You should see Prometheus-formatted metrics.

2. **Verify the target in Prometheus**:
   - Visit http://localhost:9090/targets
   - The Katana target should show as "UP"

3. **Check the port configuration**:
   - Ensure Katana metrics port matches Prometheus config
   - Default is 9100 for local Katana, 9001 in Docker

4. **For Docker Desktop (Mac/Windows)**:
   - Use `host.docker.internal` instead of `localhost` in Prometheus config
   - This allows Docker containers to access host services

5. **For Linux**:
   - Use `--network host` in Docker Compose, or
   - Use the host's IP address instead of `localhost`

### Grafana Shows No Data

1. Check that Prometheus is scraping successfully
2. Verify the time range in Grafana (top right)
3. Ensure Katana has been running long enough to generate metrics
4. Check Prometheus data source configuration in Grafana

### Dashboard Panels Missing

If the database panels don't appear:
```bash
cd monitoring
python3 add_db_panels.py
docker-compose restart grafana
```

## Development

### Adding New Panels

1. Create panels manually in Grafana UI
2. Export the dashboard JSON
3. Replace `grafana/dashboards/overview.json`

Or use the Python script `add_db_panels.py` as a template for programmatic panel generation.

### Metrics Implementation

Database metrics are implemented in `crates/storage/db/src/mdbx/metrics.rs`.

To add new metrics:
1. Add the metric to the appropriate `Metrics` struct
2. Call the recording method at the appropriate location
3. Update the dashboard to visualize the new metric

## Files

- `docker-compose.yml` - Service definitions
- `prometheus/config.yml` - Prometheus scrape configuration
- `grafana/dashboards/overview.json` - Main Grafana dashboard
- `grafana/datasources/` - Auto-provisioned data sources
- `add_db_panels.py` - Script to generate database monitoring panels
- `DATABASE_MONITORING.md` - Detailed guide to database metrics

## Resources

- [Prometheus Documentation](https://prometheus.io/docs/)
- [Grafana Documentation](https://grafana.com/docs/)
- [MDBX Documentation](https://erthink.github.io/libmdbx/)
