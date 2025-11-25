#!/usr/bin/env python3
"""
Script to add comprehensive database monitoring panels to Grafana dashboard.
This adds panels for MDBX database metrics including transactions, operations, and performance.
"""

import json
import sys

def create_row_panel(title, y_pos, panel_id):
    """Create a collapsible row panel."""
    return {
        "collapsed": False,
        "gridPos": {
            "h": 1,
            "w": 24,
            "x": 0,
            "y": y_pos
        },
        "id": panel_id,
        "panels": [],
        "title": title,
        "type": "row"
    }

def create_timeseries_panel(title, expr, legend_format, x, y, w, h, panel_id, description="", unit="short"):
    """Create a time series panel."""
    return {
        "datasource": "Prometheus",
        "description": description,
        "fieldConfig": {
            "defaults": {
                "color": {
                    "mode": "palette-classic"
                },
                "custom": {
                    "axisBorderShow": False,
                    "axisCenteredZero": False,
                    "axisColorMode": "text",
                    "axisLabel": "",
                    "axisPlacement": "auto",
                    "barAlignment": 0,
                    "drawStyle": "line",
                    "fillOpacity": 10,
                    "gradientMode": "none",
                    "hideFrom": {
                        "legend": False,
                        "tooltip": False,
                        "viz": False
                    },
                    "insertNulls": False,
                    "lineInterpolation": "smooth",
                    "lineWidth": 1,
                    "pointSize": 5,
                    "scaleDistribution": {
                        "type": "linear"
                    },
                    "showPoints": "auto",
                    "spanNulls": False,
                    "stacking": {
                        "group": "A",
                        "mode": "none"
                    },
                    "thresholdsStyle": {
                        "mode": "off"
                    }
                },
                "mappings": [],
                "thresholds": {
                    "mode": "absolute",
                    "steps": [
                        {
                            "color": "green",
                            "value": None
                        },
                        {
                            "color": "red",
                            "value": 80
                        }
                    ]
                },
                "unit": unit
            },
            "overrides": []
        },
        "gridPos": {
            "h": h,
            "w": w,
            "x": x,
            "y": y
        },
        "id": panel_id,
        "options": {
            "legend": {
                "calcs": ["last"],
                "displayMode": "table",
                "placement": "bottom",
                "showLegend": True
            },
            "tooltip": {
                "mode": "multi",
                "sort": "desc"
            }
        },
        "targets": [
            {
                "datasource": {
                    "type": "prometheus",
                    "uid": "Prometheus"
                },
                "editorMode": "code",
                "expr": expr,
                "instant": False,
                "legendFormat": legend_format,
                "range": True,
                "refId": "A"
            }
        ],
        "title": title,
        "type": "timeseries"
    }

def create_stat_panel(title, expr, x, y, w, h, panel_id, description="", unit="short", colorMode="value"):
    """Create a stat panel."""
    return {
        "datasource": "Prometheus",
        "description": description,
        "fieldConfig": {
            "defaults": {
                "color": {
                    "mode": "thresholds"
                },
                "mappings": [],
                "thresholds": {
                    "mode": "absolute",
                    "steps": [
                        {
                            "color": "green",
                            "value": None
                        },
                        {
                            "color": "red",
                            "value": 80
                        }
                    ]
                },
                "unit": unit
            },
            "overrides": []
        },
        "gridPos": {
            "h": h,
            "w": w,
            "x": x,
            "y": y
        },
        "id": panel_id,
        "options": {
            "colorMode": colorMode,
            "graphMode": "area",
            "justifyMode": "auto",
            "orientation": "auto",
            "reduceOptions": {
                "calcs": ["lastNotNull"],
                "fields": "",
                "values": False
            },
            "textMode": "auto"
        },
        "pluginVersion": "11.2.0",
        "targets": [
            {
                "datasource": {
                    "type": "prometheus",
                    "uid": "Prometheus"
                },
                "editorMode": "code",
                "expr": expr,
                "instant": True,
                "legendFormat": "__auto",
                "range": False,
                "refId": "A"
            }
        ],
        "title": title,
        "type": "stat"
    }

def create_gauge_panel(title, expr, x, y, w, h, panel_id, description="", max_value=100):
    """Create a gauge panel for percentages."""
    return {
        "datasource": "Prometheus",
        "description": description,
        "fieldConfig": {
            "defaults": {
                "color": {
                    "mode": "thresholds"
                },
                "mappings": [],
                "max": max_value,
                "min": 0,
                "thresholds": {
                    "mode": "absolute",
                    "steps": [
                        {
                            "color": "green",
                            "value": None
                        },
                        {
                            "color": "yellow",
                            "value": 70
                        },
                        {
                            "color": "red",
                            "value": 90
                        }
                    ]
                },
                "unit": "percent"
            },
            "overrides": []
        },
        "gridPos": {
            "h": h,
            "w": w,
            "x": x,
            "y": y
        },
        "id": panel_id,
        "options": {
            "minVizHeight": 75,
            "minVizWidth": 75,
            "orientation": "auto",
            "reduceOptions": {
                "calcs": ["lastNotNull"],
                "fields": "",
                "values": False
            },
            "showThresholdLabels": False,
            "showThresholdMarkers": True
        },
        "pluginVersion": "11.2.0",
        "targets": [
            {
                "datasource": {
                    "type": "prometheus",
                    "uid": "Prometheus"
                },
                "editorMode": "code",
                "expr": expr,
                "instant": True,
                "legendFormat": "__auto",
                "range": False,
                "refId": "A"
            }
        ],
        "title": title,
        "type": "gauge"
    }

def generate_db_panels():
    """Generate all database monitoring panels."""
    panels = []
    current_y = 17  # Start after existing Database row
    panel_id = 200  # Start with high ID to avoid conflicts

    # Row 1: Database Transactions
    panels.append(create_row_panel("Database Transactions", current_y, panel_id))
    panel_id += 1
    current_y += 1

    # Transaction creation rates
    panels.append(create_timeseries_panel(
        "Transaction Creation Rate",
        "rate(katana_db_transaction_ro_created[5m])",
        "Read-Only",
        0, current_y, 12, 8, panel_id,
        description="Rate of database transaction creation (read-only vs read-write)",
        unit="reqps"
    ))
    panels[-1]["targets"].append({
        "datasource": {
            "type": "prometheus",
            "uid": "Prometheus"
        },
        "editorMode": "code",
        "expr": "rate(katana_db_transaction_rw_created[5m])",
        "instant": False,
        "legendFormat": "Read-Write",
        "range": True,
        "refId": "B"
    })
    panel_id += 1

    # Transaction commit status
    panels.append(create_timeseries_panel(
        "Transaction Commit Status",
        "rate(katana_db_transaction_commits_successful[5m])",
        "Successful",
        12, current_y, 12, 8, panel_id,
        description="Rate of successful vs failed transaction commits",
        unit="reqps"
    ))
    panels[-1]["targets"].append({
        "datasource": {
            "type": "prometheus",
            "uid": "Prometheus"
        },
        "editorMode": "code",
        "expr": "rate(katana_db_transaction_commits_failed[5m])",
        "instant": False,
        "legendFormat": "Failed",
        "range": True,
        "refId": "B"
    })
    panels[-1]["targets"].append({
        "datasource": {
            "type": "prometheus",
            "uid": "Prometheus"
        },
        "editorMode": "code",
        "expr": "rate(katana_db_transaction_aborts[5m])",
        "instant": False,
        "legendFormat": "Aborted",
        "range": True,
        "refId": "C"
    })
    panel_id += 1
    current_y += 8

    # Transaction stats
    panels.append(create_stat_panel(
        "Total RO Transactions",
        "katana_db_transaction_ro_created",
        0, current_y, 6, 4, panel_id,
        description="Total number of read-only transactions created",
        unit="short"
    ))
    panel_id += 1

    panels.append(create_stat_panel(
        "Total RW Transactions",
        "katana_db_transaction_rw_created",
        6, current_y, 6, 4, panel_id,
        description="Total number of read-write transactions created",
        unit="short"
    ))
    panel_id += 1

    panels.append(create_stat_panel(
        "Successful Commits",
        "katana_db_transaction_commits_successful",
        12, current_y, 6, 4, panel_id,
        description="Total number of successful transaction commits",
        unit="short"
    ))
    panel_id += 1

    panels.append(create_stat_panel(
        "Failed/Aborted",
        "katana_db_transaction_commits_failed + katana_db_transaction_aborts",
        18, current_y, 6, 4, panel_id,
        description="Total number of failed commits and aborted transactions",
        unit="short"
    ))
    panel_id += 1
    current_y += 4

    # Row 2: Database Operations
    panels.append(create_row_panel("Database Operations", current_y, panel_id))
    panel_id += 1
    current_y += 1

    # Operation rates
    panels.append(create_timeseries_panel(
        "Operation Rate by Type",
        "rate(katana_db_operation_puts[5m])",
        "Puts",
        0, current_y, 12, 8, panel_id,
        description="Rate of database operations (get, put, delete, clear)",
        unit="reqps"
    ))
    panels[-1]["targets"].extend([
        {
            "datasource": {"type": "prometheus", "uid": "Prometheus"},
            "editorMode": "code",
            "expr": "rate(katana_db_operation_get_hits[5m]) + rate(katana_db_operation_get_misses[5m])",
            "instant": False,
            "legendFormat": "Gets (Total)",
            "range": True,
            "refId": "B"
        },
        {
            "datasource": {"type": "prometheus", "uid": "Prometheus"},
            "editorMode": "code",
            "expr": "rate(katana_db_operation_deletes_successful[5m]) + rate(katana_db_operation_deletes_failed[5m])",
            "instant": False,
            "legendFormat": "Deletes (Total)",
            "range": True,
            "refId": "C"
        },
        {
            "datasource": {"type": "prometheus", "uid": "Prometheus"},
            "editorMode": "code",
            "expr": "rate(katana_db_operation_clears[5m])",
            "instant": False,
            "legendFormat": "Clears",
            "range": True,
            "refId": "D"
        }
    ])
    panel_id += 1

    # Operation success rates
    panels.append(create_timeseries_panel(
        "Delete Success Rate",
        "rate(katana_db_operation_deletes_successful[5m])",
        "Successful",
        12, current_y, 12, 8, panel_id,
        description="Delete operation success vs failure rate",
        unit="reqps"
    ))
    panels[-1]["targets"].append({
        "datasource": {"type": "prometheus", "uid": "Prometheus"},
        "editorMode": "code",
        "expr": "rate(katana_db_operation_deletes_failed[5m])",
        "instant": False,
        "legendFormat": "Failed",
        "range": True,
        "refId": "B"
    })
    panel_id += 1
    current_y += 8

    # Cache hit rate gauge
    panels.append(create_gauge_panel(
        "Get Cache Hit Rate",
        "100 * rate(katana_db_operation_get_hits[5m]) / (rate(katana_db_operation_get_hits[5m]) + rate(katana_db_operation_get_misses[5m]))",
        0, current_y, 6, 6, panel_id,
        description="Percentage of get operations that found a value (cache hit rate)"
    ))
    panel_id += 1

    # Operation totals
    panels.append(create_stat_panel(
        "Total Gets",
        "katana_db_operation_get_hits + katana_db_operation_get_misses",
        6, current_y, 6, 3, panel_id,
        description="Total number of get operations",
        unit="short"
    ))
    panel_id += 1

    panels.append(create_stat_panel(
        "Get Hits",
        "katana_db_operation_get_hits",
        6, current_y + 3, 3, 3, panel_id,
        description="Number of successful get operations",
        unit="short",
        colorMode="background"
    ))
    panel_id += 1

    panels.append(create_stat_panel(
        "Get Misses",
        "katana_db_operation_get_misses",
        9, current_y + 3, 3, 3, panel_id,
        description="Number of get operations that didn't find a value",
        unit="short",
        colorMode="background"
    ))
    panel_id += 1

    panels.append(create_stat_panel(
        "Total Puts",
        "katana_db_operation_puts",
        12, current_y, 6, 3, panel_id,
        description="Total number of put operations",
        unit="short"
    ))
    panel_id += 1

    panels.append(create_stat_panel(
        "Total Deletes",
        "katana_db_operation_deletes_successful + katana_db_operation_deletes_failed",
        18, current_y, 6, 3, panel_id,
        description="Total number of delete operations",
        unit="short"
    ))
    panel_id += 1

    panels.append(create_stat_panel(
        "Delete Success",
        "katana_db_operation_deletes_successful",
        18, current_y + 3, 3, 3, panel_id,
        description="Number of successful delete operations",
        unit="short",
        colorMode="background"
    ))
    panel_id += 1

    panels.append(create_stat_panel(
        "Delete Failures",
        "katana_db_operation_deletes_failed",
        21, current_y + 3, 3, 3, panel_id,
        description="Number of failed delete operations",
        unit="short",
        colorMode="background"
    ))
    panel_id += 1
    current_y += 6

    # Row 3: Performance Metrics
    panels.append(create_row_panel("Database Performance", current_y, panel_id))
    panel_id += 1
    current_y += 1

    # Commit time
    panels.append(create_timeseries_panel(
        "Transaction Commit Time (p99)",
        "histogram_quantile(0.99, rate(katana_db_transaction_commit_time_seconds_bucket[5m]))",
        "p99",
        0, current_y, 12, 8, panel_id,
        description="99th percentile transaction commit time",
        unit="s"
    ))
    panels[-1]["targets"].extend([
        {
            "datasource": {"type": "prometheus", "uid": "Prometheus"},
            "editorMode": "code",
            "expr": "histogram_quantile(0.95, rate(katana_db_transaction_commit_time_seconds_bucket[5m]))",
            "instant": False,
            "legendFormat": "p95",
            "range": True,
            "refId": "B"
        },
        {
            "datasource": {"type": "prometheus", "uid": "Prometheus"},
            "editorMode": "code",
            "expr": "histogram_quantile(0.50, rate(katana_db_transaction_commit_time_seconds_bucket[5m]))",
            "instant": False,
            "legendFormat": "p50",
            "range": True,
            "refId": "C"
        }
    ])
    panel_id += 1

    # Get operation time
    panels.append(create_timeseries_panel(
        "Get Operation Time (p99)",
        "histogram_quantile(0.99, rate(katana_db_operation_get_time_seconds_bucket[5m]))",
        "p99",
        12, current_y, 12, 8, panel_id,
        description="99th percentile get operation time",
        unit="s"
    ))
    panels[-1]["targets"].extend([
        {
            "datasource": {"type": "prometheus", "uid": "Prometheus"},
            "editorMode": "code",
            "expr": "histogram_quantile(0.95, rate(katana_db_operation_get_time_seconds_bucket[5m]))",
            "instant": False,
            "legendFormat": "p95",
            "range": True,
            "refId": "B"
        },
        {
            "datasource": {"type": "prometheus", "uid": "Prometheus"},
            "editorMode": "code",
            "expr": "histogram_quantile(0.50, rate(katana_db_operation_get_time_seconds_bucket[5m]))",
            "instant": False,
            "legendFormat": "p50",
            "range": True,
            "refId": "C"
        }
    ])
    panel_id += 1
    current_y += 8

    # Put operation time
    panels.append(create_timeseries_panel(
        "Put Operation Time (p99)",
        "histogram_quantile(0.99, rate(katana_db_operation_put_time_seconds_bucket[5m]))",
        "p99",
        0, current_y, 12, 8, panel_id,
        description="99th percentile put operation time",
        unit="s"
    ))
    panels[-1]["targets"].extend([
        {
            "datasource": {"type": "prometheus", "uid": "Prometheus"},
            "editorMode": "code",
            "expr": "histogram_quantile(0.95, rate(katana_db_operation_put_time_seconds_bucket[5m]))",
            "instant": False,
            "legendFormat": "p95",
            "range": True,
            "refId": "B"
        },
        {
            "datasource": {"type": "prometheus", "uid": "Prometheus"},
            "editorMode": "code",
            "expr": "histogram_quantile(0.50, rate(katana_db_operation_put_time_seconds_bucket[5m]))",
            "instant": False,
            "legendFormat": "p50",
            "range": True,
            "refId": "C"
        }
    ])
    panel_id += 1

    # Delete operation time
    panels.append(create_timeseries_panel(
        "Delete Operation Time (p99)",
        "histogram_quantile(0.99, rate(katana_db_operation_delete_time_seconds_bucket[5m]))",
        "p99",
        12, current_y, 12, 8, panel_id,
        description="99th percentile delete operation time",
        unit="s"
    ))
    panels[-1]["targets"].extend([
        {
            "datasource": {"type": "prometheus", "uid": "Prometheus"},
            "editorMode": "code",
            "expr": "histogram_quantile(0.95, rate(katana_db_operation_delete_time_seconds_bucket[5m]))",
            "instant": False,
            "legendFormat": "p95",
            "range": True,
            "refId": "B"
        },
        {
            "datasource": {"type": "prometheus", "uid": "Prometheus"},
            "editorMode": "code",
            "expr": "histogram_quantile(0.50, rate(katana_db_operation_delete_time_seconds_bucket[5m]))",
            "instant": False,
            "legendFormat": "p50",
            "range": True,
            "refId": "C"
        }
    ])
    panel_id += 1

    return panels

def main():
    dashboard_path = "grafana/dashboards/overview.json"

    # Load existing dashboard
    with open(dashboard_path, 'r') as f:
        dashboard = json.load(f)

    # Generate new panels
    new_panels = generate_db_panels()

    # Find the Database row and insert after Freelist Size panel
    db_row_index = None
    for i, panel in enumerate(dashboard['panels']):
        if panel.get('type') == 'row' and panel.get('title') == 'Database':
            db_row_index = i
            break

    if db_row_index is None:
        print("Error: Could not find Database row")
        sys.exit(1)

    # Find the last panel in the Database section (before next row)
    insert_index = db_row_index + 1
    while insert_index < len(dashboard['panels']) and dashboard['panels'][insert_index].get('type') != 'row':
        insert_index += 1

    # Update y positions for existing panels after insertion point
    y_offset = 40  # Approximate total height of new panels
    for panel in dashboard['panels'][insert_index:]:
        if 'gridPos' in panel:
            panel['gridPos']['y'] += y_offset

    # Insert new panels
    dashboard['panels'][insert_index:insert_index] = new_panels

    # Save updated dashboard
    with open(dashboard_path, 'w') as f:
        json.dump(dashboard, f, indent=2)

    print(f"Successfully added {len(new_panels)} database monitoring panels to {dashboard_path}")
    print("\nNew panels include:")
    print("  - Database Transactions (creation rate, commit status, totals)")
    print("  - Database Operations (operation rates, success rates, cache hit rate)")
    print("  - Database Performance (commit time, get/put/delete operation times)")

if __name__ == "__main__":
    main()
