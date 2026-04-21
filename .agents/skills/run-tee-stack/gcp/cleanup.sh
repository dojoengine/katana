#!/usr/bin/env bash
# Tears down the VM + firewall rule provisioned by ./provision.sh.
# Usage: GCP_ZONE=us-central1-a STACK_NAME=katana-tee-stack ./cleanup.sh
set -euo pipefail
STACK_NAME="${STACK_NAME:-katana-tee-stack}"
: "${GCP_ZONE:?GCP_ZONE is required}"
PROJECT=$(gcloud config get-value project 2>/dev/null)

if gcloud compute instances describe "$STACK_NAME" --zone "$GCP_ZONE" --project "$PROJECT" --quiet 2>/dev/null >/dev/null; then
    echo "[gcp/cleanup] Deleting instance $STACK_NAME..."
    gcloud compute instances delete "$STACK_NAME" --zone "$GCP_ZONE" --project "$PROJECT" --quiet
fi

FW_NAME="${STACK_NAME}-allow"
if gcloud compute firewall-rules describe "$FW_NAME" --project "$PROJECT" --quiet 2>/dev/null >/dev/null; then
    echo "[gcp/cleanup] Deleting firewall rule $FW_NAME..."
    gcloud compute firewall-rules delete "$FW_NAME" --project "$PROJECT" --quiet
fi

echo "[gcp/cleanup] done."
