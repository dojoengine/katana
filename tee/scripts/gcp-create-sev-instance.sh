#!/usr/bin/env bash
#
# gcp-create-sev-instance.sh - Create a GCP AMD SEV-SNP Confidential VM for testing
#
# This script creates a GCP Confidential Computing instance with:
# - AMD SEV-SNP enabled
# - Nested virtualization enabled (for running QEMU)
# - Ubuntu 22.04 LTS
# - Sufficient resources for running Katana in a nested VM
#
# Prerequisites:
#   - gcloud CLI installed and authenticated
#   - Project with Compute Engine API enabled
#   - Billing account linked
#
# Usage:
#   ./gcp-create-sev-instance.sh [INSTANCE_NAME] [ZONE]
#

set -euo pipefail

# Configuration
INSTANCE_NAME="${1:-katana-sev-test}"
ZONE="${2:-us-central1-a}"
PROJECT_ID="${GCP_PROJECT_ID:-$(gcloud config get-value project)}"
MACHINE_TYPE="n2d-standard-4"  # AMD EPYC with 4 vCPUs, 16GB RAM
BOOT_DISK_SIZE="50GB"
IMAGE_FAMILY="ubuntu-2204-lts"
IMAGE_PROJECT="ubuntu-os-cloud"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log_info() { echo -e "${BLUE}[INFO]${NC} $*"; }
log_success() { echo -e "${GREEN}[SUCCESS]${NC} $*"; }
log_warning() { echo -e "${YELLOW}[WARNING]${NC} $*"; }
log_error() { echo -e "${RED}[ERROR]${NC} $*"; }

check_gcloud() {
    if ! command -v gcloud &> /dev/null; then
        log_error "gcloud CLI not found"
        log_info "Install from: https://cloud.google.com/sdk/docs/install"
        exit 1
    fi

    if [ -z "$PROJECT_ID" ]; then
        log_error "GCP project not set"
        log_info "Set with: gcloud config set project PROJECT_ID"
        exit 1
    fi

    log_info "Using GCP project: $PROJECT_ID"
}

check_instance_exists() {
    if gcloud compute instances describe "$INSTANCE_NAME" --zone="$ZONE" --project="$PROJECT_ID" &> /dev/null; then
        log_warning "Instance '$INSTANCE_NAME' already exists in zone '$ZONE'"
        read -p "Delete and recreate? (y/N): " -n 1 -r
        echo
        if [[ $REPLY =~ ^[Yy]$ ]]; then
            log_info "Deleting existing instance..."
            gcloud compute instances delete "$INSTANCE_NAME" \
                --zone="$ZONE" \
                --project="$PROJECT_ID" \
                --quiet
            log_success "Instance deleted"
        else
            log_info "Using existing instance"
            exit 0
        fi
    fi
}

create_instance() {
    log_info "Creating AMD SEV-SNP Confidential VM instance..."
    log_info "  Instance: $INSTANCE_NAME"
    log_info "  Zone: $ZONE"
    log_info "  Machine: $MACHINE_TYPE (AMD EPYC)"
    log_info "  Image: $IMAGE_FAMILY"
    echo ""

    gcloud compute instances create "$INSTANCE_NAME" \
        --zone="$ZONE" \
        --project="$PROJECT_ID" \
        --machine-type="$MACHINE_TYPE" \
        --boot-disk-size="$BOOT_DISK_SIZE" \
        --boot-disk-type="pd-balanced" \
        --image-family="$IMAGE_FAMILY" \
        --image-project="$IMAGE_PROJECT" \
        --confidential-compute \
        --confidential-compute-type=SEV_SNP \
        --maintenance-policy=TERMINATE \
        --min-cpu-platform="AMD Milan" \
        --enable-nested-virtualization \
        --metadata=enable-oslogin=FALSE \
        --scopes=cloud-platform \
        --tags=katana-sev-test

    log_success "Instance created successfully"
}

wait_for_instance() {
    log_info "Waiting for instance to be ready..."

    local max_wait=120
    local waited=0

    while [ $waited -lt $max_wait ]; do
        if gcloud compute ssh "$INSTANCE_NAME" \
            --zone="$ZONE" \
            --project="$PROJECT_ID" \
            --command="echo 'ready'" &> /dev/null; then
            log_success "Instance is ready"
            return 0
        fi

        sleep 5
        waited=$((waited + 5))
        echo -n "."
    done

    echo ""
    log_error "Instance did not become ready within ${max_wait}s"
    exit 1
}

show_connection_info() {
    local external_ip
    external_ip=$(gcloud compute instances describe "$INSTANCE_NAME" \
        --zone="$ZONE" \
        --project="$PROJECT_ID" \
        --format="get(networkInterfaces[0].accessConfigs[0].natIP)")

    echo ""
    echo "==========================================="
    log_success "GCP Confidential VM Created"
    echo "==========================================="
    echo ""
    echo "Instance Name: $INSTANCE_NAME"
    echo "Zone:          $ZONE"
    echo "External IP:   $external_ip"
    echo "Machine Type:  $MACHINE_TYPE"
    echo "Features:      AMD SEV-SNP, Nested Virtualization"
    echo ""
    echo "Connect with:"
    echo "  gcloud compute ssh $INSTANCE_NAME --zone=$ZONE --project=$PROJECT_ID"
    echo ""
    echo "Next steps:"
    echo "  1. Upload boot components: ./gcp-upload-components.sh $INSTANCE_NAME $ZONE"
    echo "  2. Setup instance: gcloud compute ssh $INSTANCE_NAME --zone=$ZONE -- 'bash -s' < tee/scripts/gcp-setup-instance.sh"
    echo "  3. Run attestation test: gcloud compute ssh $INSTANCE_NAME --zone=$ZONE -- 'cd katana && ./tee/scripts/gcp-test-attestation.sh'"
    echo ""
    echo "Cleanup:"
    echo "  gcloud compute instances delete $INSTANCE_NAME --zone=$ZONE --project=$PROJECT_ID"
    echo ""
}

main() {
    echo "==========================================="
    echo "GCP AMD SEV-SNP Confidential VM Creator"
    echo "==========================================="
    echo ""

    check_gcloud
    check_instance_exists
    create_instance
    wait_for_instance
    show_connection_info
}

if [ "${1:-}" = "--help" ] || [ "${1:-}" = "-h" ]; then
    cat <<EOF
Usage: $0 [INSTANCE_NAME] [ZONE]

Create a GCP Confidential Computing instance with AMD SEV-SNP for testing.

Arguments:
  INSTANCE_NAME    Name for the instance (default: katana-sev-test)
  ZONE            GCP zone (default: us-central1-a)

Environment:
  GCP_PROJECT_ID  GCP project ID (default: from gcloud config)

Examples:
  # Create with defaults
  $0

  # Create with custom name and zone
  $0 my-katana-test europe-west4-a

  # Set project explicitly
  GCP_PROJECT_ID=my-project $0

Available AMD EPYC Zones:
  us-central1-a, us-central1-b, us-central1-c
  us-east1-b, us-east1-c, us-east1-d
  europe-west4-a, europe-west4-b, europe-west4-c
  asia-southeast1-a, asia-southeast1-b, asia-southeast1-c

Machine Types (AMD EPYC):
  n2d-standard-2  (2 vCPU, 8GB RAM)
  n2d-standard-4  (4 vCPU, 16GB RAM) <- Recommended
  n2d-standard-8  (8 vCPU, 32GB RAM)

Cost Estimate:
  n2d-standard-4: ~$0.15/hour (~$3.60/day)
  Boot disk 50GB: ~$0.08/day
  Total: ~$4/day (remember to delete when done!)
EOF
    exit 0
fi

main "$@"
