#!/bin/bash
set -euo pipefail

# Test VM Boot with QEMU
# Boots the Katana VM image and validates it works correctly

OVMF=${1:-tee/output/ovmf.fd}
TIMEOUT=${3:-60}
MEMORY=${4:-4G}
VCPUS=${5:-4}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

echo "=========================================="
echo "Katana VM Boot Test"
echo "=========================================="
echo "OVMF:    $OVMF"
echo "Memory:  $MEMORY"
echo "VCPUs:   $VCPUS"
echo "Timeout: ${TIMEOUT}s"
echo "=========================================="
echo ""

# Verify files exist
if [[ ! -f "$OVMF" ]]; then
    echo "ERROR: OVMF file not found: $OVMF"
    exit 1
fi


# Check if QEMU is installed
if ! command -v qemu-system-x86_64 &> /dev/null; then
    echo "ERROR: qemu-system-x86_64 not found"
    echo "Install with: sudo apt-get install qemu-system-x86"
    exit 1
fi

echo "✓ All prerequisites met"
echo ""

# Create a working copy of the disk (QEMU will modify it)
WORK_DIR=$(mktemp -d)
# WORK_DISK="$WORK_DIR/disk.raw"
# cp "$DISK" "$WORK_DISK"

# Cleanup function
cleanup() {
    echo ""
    echo "Cleaning up..."
    if [[ -n "${QEMU_PID:-}" ]] && kill -0 "$QEMU_PID" 2>/dev/null; then
        echo "Stopping QEMU (PID: $QEMU_PID)..."
        kill "$QEMU_PID" 2>/dev/null || true
        sleep 2
        kill -9 "$QEMU_PID" 2>/dev/null || true
    fi
    rm -rf "$WORK_DIR"
    echo "✓ Cleanup complete"
}

trap cleanup EXIT

# Start QEMU in background with serial output to file
SERIAL_LOG="$WORK_DIR/serial.log"
echo "Starting QEMU..."
echo "Serial output will be logged to: $SERIAL_LOG"
echo ""

# Run QEMU with:
# - Direct kernel boot (faster than UEFI boot)
# - 4 CPUs, 4G RAM (required for 60MB initrd)
# - Serial console redirected to file
# - No graphics, no network
/usr/bin/qemu-system-x86_64 \
    -m "$MEMORY" \
    -smp "$VCPUS" \
    -kernel "$PROJECT_ROOT/tee/output/vmlinuz" \
    -initrd "$PROJECT_ROOT/tee/output/initrd.img" \
    -append "console=ttyS0" \
    -nographic \
    -serial file:"$SERIAL_LOG" \
    > /dev/null 2>&1 &

QEMU_PID=$!
echo "✓ QEMU started (PID: $QEMU_PID)"
echo ""

# Wait for QEMU to start
sleep 2

if ! kill -0 "$QEMU_PID" 2>/dev/null; then
    echo "ERROR: QEMU failed to start or exited immediately"
    cat "$SERIAL_LOG"
    exit 1
fi

echo "Waiting for boot (timeout: ${TIMEOUT}s)..."
echo "Monitoring serial output for Katana initialization..."
echo ""

# Monitor serial output for signs of boot progress
START_TIME=$(date +%s)
BOOT_STARTED=false
INIT_STARTED=false
KATANA_STARTED=false

while true; do
    CURRENT_TIME=$(date +%s)
    ELAPSED=$((CURRENT_TIME - START_TIME))

    if [[ $ELAPSED -gt $TIMEOUT ]]; then
        echo ""
        echo "ERROR: Timeout after ${TIMEOUT}s"
        echo ""
        echo "Serial output:"
        cat "$SERIAL_LOG"
        exit 1
    fi

    # Check if QEMU is still running
    if ! kill -0 "$QEMU_PID" 2>/dev/null; then
        echo ""
        echo "ERROR: QEMU exited unexpectedly"
        echo ""
        echo "Serial output:"
        cat "$SERIAL_LOG"
        exit 1
    fi

    # Check serial log for boot progress
    if [[ -f "$SERIAL_LOG" ]]; then
        # Look for kernel boot messages
        if ! $BOOT_STARTED && grep -q "Linux version" "$SERIAL_LOG" 2>/dev/null; then
            BOOT_STARTED=true
            echo "✓ Kernel boot detected (${ELAPSED}s)"
        fi

        # Look for init script starting
        if ! $INIT_STARTED && grep -q "Katana TEE Init" "$SERIAL_LOG" 2>/dev/null; then
            INIT_STARTED=true
            echo "✓ Init script started (${ELAPSED}s)"
        fi

        # Look for Katana launching
        if ! $KATANA_STARTED && grep -q "Launching Katana" "$SERIAL_LOG" 2>/dev/null; then
            KATANA_STARTED=true
            echo "✓ Katana launch detected (${ELAPSED}s)"
            # Give Katana a bit more time to fully start
            sleep 5
            break
        fi

        # Alternative: look for any error messages
        if grep -q "Kernel panic" "$SERIAL_LOG" 2>/dev/null; then
            echo ""
            echo "ERROR: Kernel panic detected"
            echo ""
            echo "Serial output:"
            cat "$SERIAL_LOG"
            exit 1
        fi
    fi

    # Progress indicator
    if [[ $((ELAPSED % 5)) -eq 0 ]]; then
        echo "  ... waiting (${ELAPSED}s)"
    fi

    sleep 1
done

echo ""
echo "=========================================="
echo "✓ VM Boot Test PASSED"
echo "=========================================="
echo "Total boot time: ${ELAPSED}s"
echo ""
echo "Boot stages:"
echo "  - Kernel boot:   ✓"
echo "  - Init script:   ✓"
echo "  - Katana launch: ✓"
echo ""
echo "Serial output (last 50 lines):"
echo "------------------------------------------"
tail -n 50 "$SERIAL_LOG"
echo "------------------------------------------"
echo ""
echo "Full serial log saved to: $SERIAL_LOG"
echo "(Copying to ./vm-boot-test.log)"
cp "$SERIAL_LOG" ./vm-boot-test.log

echo ""
echo "NOTE: QEMU is still running in the background."
echo "Press Ctrl+C to stop, or it will be stopped on script exit."
echo ""
echo "To keep QEMU running after this script, run:"
echo "  kill -9 $QEMU_PID  # to force stop"
echo ""

# Keep running so user can examine
echo "Waiting 10 seconds before cleanup..."
sleep 10
