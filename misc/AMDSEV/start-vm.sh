#!/bin/bash
# Start TEE VM with AMD SEV-SNP
# Usage: ./start-vm.sh [BOOT_COMPONENTS_DIR]
#
# This script:
# 1. Starts QEMU with the TEE boot components
# 2. Runs katana without persistent storage (in-memory only)
# 3. Forwards RPC port to host
#
# ==============================================================================
# LAUNCH MEASUREMENT INPUTS
# ==============================================================================
# The following parameters are used by QEMU/OVMF to compute the SEV-SNP launch
# measurement. Verifiers must use the same values to reproduce the measurement.
#
# Boot components (hashed when kernel-hashes=on):
#   OVMF_FILE        - OVMF.fd firmware image
#   KERNEL_FILE      - vmlinuz kernel image
#   INITRD_FILE      - initrd.img initial ramdisk
#   KERNEL_CMDLINE   - "console=ttyS0 katana.args=--http.addr,0.0.0.0,--http.port,5050,--tee.provider,sev-snp"
#
# SEV-SNP guest configuration:
#   GUEST_POLICY     - 0x30000 (SMT allowed, debug disabled)
#   VCPU_COUNT       - 1
#   VMSA_FEATURES    - 0x1 (SNP active)
#
# CPU and platform:
#   CPU_TYPE         - EPYC-v4
#   CBITPOS          - 51 (C-bit position for memory encryption)
#   REDUCED_PHYS_BITS - 1
#
# To compute expected measurement, use snp-digest from snp-tools:
#   cargo build -p snp-tools
#   ./target/debug/snp-digest --ovmf=OVMF.fd --kernel=vmlinuz --initrd=initrd.img \
#       --append="console=ttyS0 katana.args=--http.addr,0.0.0.0,--http.port,5050,--tee.provider,sev-snp" \
#       --vcpus=1 --cpu=epyc-v4 --vmm=qemu --guest-features=0x1
#
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BOOT_DIR="${1:-$SCRIPT_DIR/output/qemu}"

# ------------------------------------------------------------------------------
# Launch measurement inputs (must match values documented above)
# ------------------------------------------------------------------------------

# Boot components
OVMF_FILE="$BOOT_DIR/OVMF.fd"
KERNEL_FILE="$BOOT_DIR/vmlinuz"
INITRD_FILE="$BOOT_DIR/initrd.img"
KERNEL_CMDLINE="console=ttyS0 katana.args=--http.addr,0.0.0.0,--http.port,5050,--tee.provider,sev-snp,--fork.block,6177441,--fork.provider,https://pathfinder-sepolia.d.karnot.xyz/"

# SEV-SNP guest configuration
GUEST_POLICY="0x30000"
VCPU_COUNT=1
CBITPOS=51
REDUCED_PHYS_BITS=1

# VM resources
MEMORY="512M"
CPU_TYPE="EPYC-v4"

# Networking
KATANA_RPC_PORT=5050
HOST_RPC_PORT=15051

# Create random temp file for serial log
SERIAL_LOG=$(mktemp /tmp/katana-tee-vm-serial.XXXXXX.log)

# Cleanup function
QEMU_PID=""
cleanup() {
    local exit_code=$?

    echo ""
    echo "=== Cleanup ==="

    if [ -n "$QEMU_PID" ] && kill -0 "$QEMU_PID" 2>/dev/null; then
        echo "Stopping QEMU (PID $QEMU_PID)..."
        kill "$QEMU_PID" 2>/dev/null || true
        # Wait for QEMU to fully terminate
        for i in $(seq 1 10); do
            if ! kill -0 "$QEMU_PID" 2>/dev/null; then
                break
            fi
            sleep 0.5
        done
        # Force kill if still running
        if kill -0 "$QEMU_PID" 2>/dev/null; then
            echo "Force killing QEMU..."
            kill -9 "$QEMU_PID" 2>/dev/null || true
        fi
        wait "$QEMU_PID" 2>/dev/null || true
    fi

    # Clean up serial log file
    if [ -f "$SERIAL_LOG" ]; then
        rm -f "$SERIAL_LOG"
    fi

    echo "=== Cleanup complete ==="
    exit $exit_code
}
trap cleanup EXIT INT TERM

# Check for root/sudo (needed for KVM access)
if [ "$EUID" -ne 0 ]; then
    echo "This script requires root privileges for KVM access."
    echo "Please run with: sudo $0 $*"
    exit 1
fi

# Verify files exist
echo "Checking TEE boot components..."
for file in "$OVMF_FILE" "$KERNEL_FILE" "$INITRD_FILE"; do
    if [ ! -f "$file" ]; then
        echo "Error: Missing $file"
        exit 1
    fi
    echo "  Found: $file ($(ls -lh "$file" | awk '{print $5}'))"
done

echo ""
echo "Starting TEE QEMU VM..."
echo "  OVMF:    $OVMF_FILE"
echo "  Kernel:  $KERNEL_FILE"
echo "  Initrd:  $INITRD_FILE"
echo "  Cmdline: $KERNEL_CMDLINE"
echo "  Policy:  $GUEST_POLICY"
echo "  vCPUs:   $VCPU_COUNT"
echo "  Memory:  $MEMORY"
echo "  Serial:  $SERIAL_LOG"
echo "  RPC:     localhost:$HOST_RPC_PORT -> VM:$KATANA_RPC_PORT"
echo ""
echo "To compute expected launch measurement:"
echo "  snp-digest --ovmf=$OVMF_FILE --kernel=$KERNEL_FILE --initrd=$INITRD_FILE \\"
echo "      --append='$KERNEL_CMDLINE' --vcpus=$VCPU_COUNT --cpu=epyc-v4 --vmm=qemu --guest-features=0x1"

qemu-system-x86_64 \
    -enable-kvm \
    -cpu "$CPU_TYPE" \
    -smp "$VCPU_COUNT" \
    -m "$MEMORY" \
    -machine q35,confidential-guest-support=sev0,vmport=off \
    -object memory-backend-memfd,id=ram1,size="$MEMORY",share=true,prealloc=false \
    -machine memory-backend=ram1 \
    -object sev-snp-guest,id=sev0,policy="$GUEST_POLICY",cbitpos="$CBITPOS",reduced-phys-bits="$REDUCED_PHYS_BITS",kernel-hashes=on \
    -nographic \
    -serial "file:$SERIAL_LOG" \
    -bios "$OVMF_FILE" \
    -kernel "$KERNEL_FILE" \
    -initrd "$INITRD_FILE" \
    -append "$KERNEL_CMDLINE" \
    -netdev user,id=net0,hostfwd=tcp::${HOST_RPC_PORT}-:${KATANA_RPC_PORT} \
    -device virtio-net-pci,disable-legacy=on,iommu_platform=true,netdev=net0,romfile= \
    &

QEMU_PID=$!
echo "QEMU started with PID $QEMU_PID"

# Wait for serial log file to be created
echo ""
echo "Waiting for serial log file..."
while [ ! -f "$SERIAL_LOG" ]; do
    if ! kill -0 "$QEMU_PID" 2>/dev/null; then
        echo "Error: QEMU process died before creating serial log"
        exit 1
    fi
    sleep 0.1
done
echo "Serial log file created"

echo ""
echo "=== Following serial output (Ctrl+C to exit) ==="
tail -f "$SERIAL_LOG"
