#!/usr/bin/env python3
"""
Script to add a transaction creation rate graph to the Grafana dashboard.
This shows the rate of transaction creation (both RO and RW) over time.
"""

import json
import sys

def create_tx_creation_panel(panel_id=300, x=0, y=1, w=24, h=8):
    """Create a comprehensive transaction creation panel."""
    return {
        "datasource": "Prometheus",
        "description": "Transaction creation over time. Shows when read-only (RO) and read-write (RW) transactions are being created (number of new transactions in each time bucket).",
        "fieldConfig": {
            "defaults": {
                "color": {
                    "mode": "palette-classic"
                },
                "custom": {
                    "axisBorderShow": False,
                    "axisCenteredZero": False,
                    "axisColorMode": "text",
                    "axisLabel": "Transactions created",
                    "axisPlacement": "auto",
                    "barAlignment": 0,
                    "drawStyle": "line",
                    "fillOpacity": 15,
                    "gradientMode": "opacity",
                    "hideFrom": {
                        "legend": False,
                        "tooltip": False,
                        "viz": False
                    },
                    "insertNulls": False,
                    "lineInterpolation": "smooth",
                    "lineWidth": 2,
                    "pointSize": 5,
                    "scaleDistribution": {
                        "type": "linear"
                    },
                    "showPoints": "never",
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
                "unit": "short"
            },
            "overrides": [
                {
                    "matcher": {
                        "id": "byName",
                        "options": "Read-Only (RO)"
                    },
                    "properties": [
                        {
                            "id": "color",
                            "value": {
                                "fixedColor": "blue",
                                "mode": "fixed"
                            }
                        }
                    ]
                },
                {
                    "matcher": {
                        "id": "byName",
                        "options": "Read-Write (RW)"
                    },
                    "properties": [
                        {
                            "id": "color",
                            "value": {
                                "fixedColor": "orange",
                                "mode": "fixed"
                            }
                        }
                    ]
                },
                {
                    "matcher": {
                        "id": "byName",
                        "options": "Total"
                    },
                    "properties": [
                        {
                            "id": "color",
                            "value": {
                                "fixedColor": "purple",
                                "mode": "fixed"
                            }
                        },
                        {
                            "id": "custom.lineStyle",
                            "value": {
                                "dash": [10, 10],
                                "fill": "dash"
                            }
                        },
                        {
                            "id": "custom.fillOpacity",
                            "value": 0
                        }
                    ]
                }
            ]
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
                "calcs": [
                    "last",
                    "mean",
                    "max"
                ],
                "displayMode": "table",
                "placement": "bottom",
                "showLegend": True,
                "sortBy": "Last",
                "sortDesc": True
            },
            "tooltip": {
                "mode": "multi",
                "sort": "desc"
            }
        },
        "pluginVersion": "11.2.0",
        "targets": [
            {
                "datasource": {
                    "type": "prometheus",
                    "uid": "Prometheus"
                },
                "editorMode": "code",
                "expr": "increase(katana_db_transaction_ro_created[$__rate_interval])",
                "instant": False,
                "legendFormat": "Read-Only (RO)",
                "range": True,
                "refId": "A"
            },
            {
                "datasource": {
                    "type": "prometheus",
                    "uid": "Prometheus"
                },
                "editorMode": "code",
                "expr": "increase(katana_db_transaction_rw_created[$__rate_interval])",
                "instant": False,
                "legendFormat": "Read-Write (RW)",
                "range": True,
                "refId": "B"
            },
            {
                "datasource": {
                    "type": "prometheus",
                    "uid": "Prometheus"
                },
                "editorMode": "code",
                "expr": "increase(katana_db_transaction_ro_created[$__rate_interval]) + increase(katana_db_transaction_rw_created[$__rate_interval])",
                "instant": False,
                "legendFormat": "Total",
                "range": True,
                "refId": "C"
            }
        ],
        "title": "Transaction Creation",
        "type": "timeseries"
    }

def main():
    dashboard_path = "grafana/dashboards/overview.json"

    # Load existing dashboard
    with open(dashboard_path, 'r') as f:
        dashboard = json.load(f)

    # Find the Database Transactions row
    db_tx_row_index = None
    for i, panel in enumerate(dashboard['panels']):
        if panel.get('type') == 'row' and panel.get('title') == 'Database Transactions':
            db_tx_row_index = i
            break

    if db_tx_row_index is None:
        print("Error: Could not find 'Database Transactions' row")
        sys.exit(1)

    # Check if panel already exists
    panel_exists = False
    for panel in dashboard['panels']:
        if panel.get('id') == 300:
            panel_exists = True
            print("Transaction creation panel already exists (ID 300)")
            break

    if panel_exists:
        # Update existing panel
        for i, panel in enumerate(dashboard['panels']):
            if panel.get('id') == 300:
                # Get current position
                y_pos = panel['gridPos']['y']
                dashboard['panels'][i] = create_tx_creation_panel(
                    panel_id=300,
                    x=0,
                    y=y_pos,
                    w=24,
                    h=8
                )
                print(f"Updated existing transaction creation panel at position y={y_pos}")
                break
    else:
        # Insert new panel right after the Database Transactions row
        insert_index = db_tx_row_index + 1

        # Get the y position from the row
        row_y = dashboard['panels'][db_tx_row_index]['gridPos']['y']
        new_panel_y = row_y + 1

        # Update y positions for panels after insertion point
        y_offset = 8  # Height of the new panel
        for panel in dashboard['panels'][insert_index:]:
            if 'gridPos' in panel:
                if panel['gridPos']['y'] >= new_panel_y:
                    panel['gridPos']['y'] += y_offset

        # Create and insert new panel
        new_panel = create_tx_creation_panel(
            panel_id=300,
            x=0,
            y=new_panel_y,
            w=24,
            h=8
        )
        dashboard['panels'].insert(insert_index, new_panel)
        print(f"Added new transaction creation panel at position y={new_panel_y}")

    # Save updated dashboard
    with open(dashboard_path, 'w') as f:
        json.dump(dashboard, f, indent=2)

    print(f"\nSuccessfully updated {dashboard_path}")
    print("\nPanel features:")
    print("  - Shows when Read-Only (RO) and Read-Write (RW) transactions are created")
    print("  - Displays actual transaction count increase (not rate)")
    print("  - Includes Total transaction creation (dashed line)")
    print("  - Line chart visualization")
    print("  - Legend shows Last, Mean, and Max values")
    print("  - Uses automatic rate interval for accurate counts")
    print("\nRestart Grafana to see changes:")
    print("  docker-compose restart grafana")

if __name__ == "__main__":
    main()
