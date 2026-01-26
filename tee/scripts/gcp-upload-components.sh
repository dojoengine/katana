#!/usr/bin/env bash
#
# gcp-upload-components.sh - Upload boot components to GCP instance
#
# This script uploads the built boot components to the GCP Confidential VM:
# - vmlinuz (kernel)
# - initrd.img (contains Katana binary)
# - ovmf.fd (UEFI firmware)
# - expected-measurement.txt
# - verify-attestation.sh script
#
# Usage:
#   ./gcp-upload-components.sh [INSTANCE_NAME] [ZONE]
#

set -euo pipefail

# Configuration
INSTANCE_NAME="${1:-katana-sev-test}"
ZONE="${2:-us-central1-a}"
PROJECT_ID="${GCP_PROJECT_ID:-$(gcloud config get-value project)}"
REMOTE_DIR="katana-sev-test"

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

check_files() {
    log_info "Checking local boot components..."

    local missing_files=()

    if [ ! -f "output/vmlinuz" ]; then
        missing_files+=("output/vmlinuz")
    fi

    if [ ! -f "output/initrd.img" ]; then
        missing_files+=("output/initrd.img")
    fi

    if [ ! -f "output/ovmf.fd" ]; then
        missing_files+=("output/ovmf.fd")
    fi

    if [ ! -f "expected-measurement.txt" ]; then
        missing_files+=("expected-measurement.txt")
    fi

    if [ ${#missing_files[@]} -gt 0 ]; then
        log_error "Missing boot components: ${missing_files[*]}"
        log_info "Build them first:"
        log_info "  1. Build reproducible binary"
        log_info "  2. Build VM image"
        log_info "  3. Calculate measurement"
        exit 1
    fi

    log_success "All boot components found"

    # Show file sizes
    log_info "Component sizes:"
    ls -lh output/vmlinuz output/initrd.img output/ovmf.fd | awk '{print "  " $9 ": " $5}'
}

upload_components() {
    log_info "Uploading boot components to $INSTANCE_NAME..."

    # Create remote directory
    gcloud compute ssh "$INSTANCE_NAME" \
        --zone="$ZONE" \
        --project="$PROJECT_ID" \
        --command="mkdir -p ~/$REMOTE_DIR" \
        || {
            log_error "Failed to connect to instance"
            log_info "Make sure the instance is running and you can connect"
            exit 1
        }

    # Upload boot components
    log_info "Uploading kernel..."
    gcloud compute scp output/vmlinuz \
        "$INSTANCE_NAME:~/$REMOTE_DIR/" \
        --zone="$ZONE" \
        --project="$PROJECT_ID"

    log_info "Uploading initrd..."
    gcloud compute scp output/initrd.img \
        "$INSTANCE_NAME:~/$REMOTE_DIR/" \
        --zone="$ZONE" \
        --project="$PROJECT_ID"

    log_info "Uploading OVMF firmware..."
    gcloud compute scp output/ovmf.fd \
        "$INSTANCE_NAME:~/$REMOTE_DIR/" \
        --zone="$ZONE" \
        --project="$PROJECT_ID"

    log_info "Uploading expected measurement..."
    gcloud compute scp expected-measurement.txt \
        "$INSTANCE_NAME:~/$REMOTE_DIR/" \
        --zone="$ZONE" \
        --project="$PROJECT_ID"

    # Upload scripts
    log_info "Uploading verification script..."
    gcloud compute scp tee/scripts/verify-attestation.sh \
        "$INSTANCE_NAME:~/$REMOTE_DIR/" \
        --zone="$ZONE" \
        --project="$PROJECT_ID"

    log_success "Upload complete"
}

create_launch_script() {
    log_info "Creating QEMU launch script on remote instance..."

    gcloud compute ssh "$INSTANCE_NAME" \
        --zone="$ZONE" \
        --project="$PROJECT_ID" \
        --command="cat > ~/$REMOTE_DIR/launch-qemu.sh" <<'EOF'
#!/usr/bin/env bash
#
# launch-qemu.sh - Launch Katana in nested SEV-SNP VM
#

set -euo pipefail

# Configuration
MEMORY="4G"
CPUS="4"
CMDLINE="console=ttyS0 katana.args=--http.addr=0.0.0.0 katana.args=--tee.provider katana.args=sev-snp"

echo "==========================================="
echo "Launching Katana in SEV-SNP VM"
echo "==========================================="
echo ""
echo "Configuration:"
echo "  Memory: $MEMORY"
echo "  CPUs: $CPUS"
echo "  Kernel: vmlinuz"
echo "  Initrd: initrd.img"
echo "  OVMF: ovmf.fd"
echo "  Cmdline: $CMDLINE"
echo ""

# Check if components exist
for file in vmlinuz initrd.img ovmf.fd; do
    if [ ! -f "$file" ]; then
        echo "[ERROR] Missing $file"
        exit 1
    fi
done

echo "[INFO] Starting QEMU with SEV-SNP..."
echo "[INFO] Katana RPC will be available on http://localhost:5050"
echo "[INFO] Press Ctrl+A then X to exit QEMU"
echo ""

# Launch QEMU with SEV-SNP
# Note: On GCP nested virtualization, SEV-SNP may not be fully functional
# but we can still test the boot process and measurement extraction
sudo qemu-system-x86_64 \
    -enable-kvm \
    -cpu host \
    -machine q35,confidential-guest-support=sev0,memory-backend=ram1 \
    -object memory-backend-memfd,id=ram1,size=$MEMORY,share=true \
    -object sev-snp-guest,id=sev0,cbitpos=51,reduced-phys-bits=1 \
    -smp $CPUS \
    -m $MEMORY \
    -bios ovmf.fd \
    -kernel vmlinuz \
    -initrd initrd.img \
    -append "$CMDLINE" \
    -nographic \
    -net nic,model=virtio \
    -net user,hostfwd=tcp::5050-:5050 \
    -serial file:/tmp/katana-sev.log \
    2>&1 | tee /tmp/qemu-launch.log
EOF

    gcloud compute ssh "$INSTANCE_NAME" \
        --zone="$ZONE" \
        --project="$PROJECT_ID" \
        --command="chmod +x ~/$REMOTE_DIR/launch-qemu.sh"

    log_success "Launch script created"
}

show_next_steps() {
    echo ""
    echo "==========================================="
    log_success "Upload Complete"
    echo "==========================================="
    echo ""
    echo "Boot components uploaded to: ~/$REMOTE_DIR/"
    echo ""
    echo "Next steps:"
    echo ""
    echo "1. Connect to the instance:"
    echo "   gcloud compute ssh $INSTANCE_NAME --zone=$ZONE --project=$PROJECT_ID"
    echo ""
    echo "2. Launch QEMU (on the instance):"
    echo "   cd ~/$REMOTE_DIR"
    echo "   sudo ./launch-qemu.sh"
    echo ""
    echo "3. In another terminal, verify attestation:"
    echo "   gcloud compute ssh $INSTANCE_NAME --zone=$ZONE --project=$PROJECT_ID"
    echo "   cd ~/$REMOTE_DIR"
    echo "   ./verify-attestation.sh"
    echo ""
    echo "Note: On GCP nested virtualization, the actual SEV-SNP measurement"
    echo "      extraction may fail because the nested guest doesn't have access"
    echo "      to /dev/sev-guest. The test validates the boot process and"
    echo "      RPC functionality. Full attestation testing requires bare metal."
    echo ""
}

main() {
    echo "==========================================="
    echo "Upload Boot Components to GCP"
    echo "==========================================="
    echo ""

    check_files
    upload_components
    create_launch_script
    show_next_steps
}

if [ "${1:-}" = "--help" ] || [ "${1:-}" = "-h" ]; then
    cat <<EOF
Usage: $0 [INSTANCE_NAME] [ZONE]

Upload boot components to GCP Confidential VM instance.

Arguments:
  INSTANCE_NAME    Name of the GCP instance (default: katana-sev-test)
  ZONE            GCP zone (default: us-central1-a)

Environment:
  GCP_PROJECT_ID  GCP project ID (default: from gcloud config)

Prerequisites:
  - Boot components built in output/ directory
  - expected-measurement.txt calculated
  - GCP instance created and running

Example:
  # Upload to default instance
  $0

  # Upload to custom instance
  $0 my-instance europe-west4-a
EOF
    exit 0
fi

main "$@"
