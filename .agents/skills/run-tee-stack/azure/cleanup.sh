#!/usr/bin/env bash
# Tears down the resource group provisioned by ./provision.sh.
# Usage: STACK_NAME=katana-tee-stack ./cleanup.sh
set -euo pipefail
STACK_NAME="${STACK_NAME:-katana-tee-stack}"
if az group show --name "$STACK_NAME" >/dev/null 2>&1; then
    echo "[azure/cleanup] Deleting resource group $STACK_NAME (no-wait)..."
    az group delete --name "$STACK_NAME" --yes --no-wait
else
    echo "[azure/cleanup] No resource group named $STACK_NAME — nothing to do."
fi
