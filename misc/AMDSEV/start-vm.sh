#!/bin/bash
# Start TEE VM with AMD SEV-SNP
# Usage: ./start-vm.sh [BOOT_COMPONENTS_DIR] [--katana-args CSV] [--no-start]
#
# This script:
# 1. Starts QEMU with the TEE boot components
# 2. Creates and attaches a data disk as /dev/sda
# 3. Optionally starts Katana asynchronously via a virtio-serial control channel
# 4. Forwards RPC port to host
#
# ==============================================================================
# LAUNCH MEASUREMENT INPUTS
# ==============================================================================
# The following parameters are used by QEMU/OVMF to compute the SEV-SNP launch
# measurement. Verifiers must use the same values to reproduce the measurement.
#
# Boot components (hashed when kernel-hashes=on):
#   OVMF_FILE      - OVMF.fd firmware image
#   KERNEL_FILE    - vmlinuz kernel image
#   INITRD_FILE    - initrd.img initial ramdisk
#   KERNEL_CMDLINE - "console=ttyS0"
#
# SEV-SNP guest configuration:
#   GUEST_POLICY      - 0x30000 (SMT allowed, debug disabled)
#   VCPU_COUNT        - 1
#   GUEST_FEATURES    - 0x1 (SNP active)
#
# CPU and platform:
#   CPU_TYPE          - EPYC-v4
#   CBITPOS           - 51 (C-bit position for memory encryption)
#   REDUCED_PHYS_BITS - 1
#
# Katana launch arguments are sent after boot over a control channel and are NOT
# part of the measured kernel command line.
#
# To compute expected measurement, use snp-digest from snp-tools:
#   cargo build -p snp-tools
#   ./target/debug/snp-digest --ovmf=OVMF.fd --kernel=vmlinuz --initrd=initrd.img \
#       --append="console=ttyS0" --vcpus=1 --cpu=epyc-v4 --vmm=qemu --guest-features=0x1
#
# ==============================================================================

set -euo pipefail

usage() {
    echo "Usage: $0 [BOOT_COMPONENTS_DIR] [--katana-args CSV] [--no-start]"
    echo ""
    echo "Starts a SEV-SNP VM and launches Katana asynchronously via control channel."
    echo ""
    echo "Arguments:"
    echo "  BOOT_COMPONENTS_DIR  Optional path containing OVMF.fd, vmlinuz, initrd.img"
    echo ""
    echo "Options:"
    echo "  --katana-args CSV    Comma-separated Katana CLI args sent after boot"
    echo "  --no-start           Boot VM without sending Katana start command"
    echo "  -h, --help           Show this help"
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BOOT_DIR="${SCRIPT_DIR}/output/qemu"
KATANA_ARGS_CSV="--http.addr,0.0.0.0,--http.port,5050,--tee.provider,sev-snp"
AUTO_START_KATANA=1

while [[ $# -gt 0 ]]; do
    case "$1" in
        --katana-args)
            [[ $# -ge 2 ]] || {
                echo "Error: --katana-args requires a value"
                exit 1
            }
            KATANA_ARGS_CSV="$2"
            shift 2
            ;;

        --no-start)
            AUTO_START_KATANA=0
            shift
            ;;

        -h|--help)
            usage
            exit 0
            ;;

        -*)
            echo "Error: Unknown option: $1"
            echo ""
            usage
            exit 1
            ;;

        *)
            BOOT_DIR="$1"
            shift
            ;;
    esac
done

# ------------------------------------------------------------------------------
# Launch measurement inputs (must match values documented above)
# ------------------------------------------------------------------------------

# Boot components
OVMF_FILE="$BOOT_DIR/OVMF.fd"
KERNEL_FILE="$BOOT_DIR/vmlinuz"
INITRD_FILE="$BOOT_DIR/initrd.img"
KERNEL_CMDLINE="console=ttyS0"

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

# Katana control channel
CONTROL_PORT_NAME="org.katana.control.0"
CONTROL_SOCKET="/tmp/katana-tee-vm-control.$$.sock"
CONTROL_TIMEOUT=60

# VM data disk (required by init script)
DISK_IMAGE="$(mktemp /tmp/katana-tee-vm-data.XXXXXX.img)"
DISK_SIZE_MB=1024

# Logs
SERIAL_LOG="$(mktemp /tmp/katana-tee-vm-serial.XXXXXX.log)"

show_serial_tail() {
    echo ""
    echo "=== Serial output (last 80 lines) ==="
    tail -80 "$SERIAL_LOG" 2>/dev/null || echo "(no output)"
}

send_control_command() {
    local cmd="$1"
    local response

    response="$(printf '%s\n' "$cmd" | socat -t 2 -T 2 - UNIX-CONNECT:"$CONTROL_SOCKET" 2>/dev/null | head -n1 || true)"
    [[ -n "$response" ]] || return 1
    echo "$response"
}

# Cleanup function
QEMU_PID=""
cleanup() {
    local exit_code=$?

    echo ""
    echo "=== Cleanup ==="

    if [[ -n "$QEMU_PID" ]] && kill -0 "$QEMU_PID" 2>/dev/null; then
        echo "Stopping QEMU (PID $QEMU_PID)..."
        kill "$QEMU_PID" 2>/dev/null || true
        for _ in $(seq 1 10); do
            if ! kill -0 "$QEMU_PID" 2>/dev/null; then
                break
            fi
            sleep 0.5
        done
        if kill -0 "$QEMU_PID" 2>/dev/null; then
            echo "Force killing QEMU..."
            kill -9 "$QEMU_PID" 2>/dev/null || true
        fi
        wait "$QEMU_PID" 2>/dev/null || true
    fi

    [[ -f "$SERIAL_LOG" ]] && rm -f "$SERIAL_LOG"
    [[ -S "$CONTROL_SOCKET" ]] && rm -f "$CONTROL_SOCKET"
    [[ -f "$DISK_IMAGE" ]] && rm -f "$DISK_IMAGE"

    echo "=== Cleanup complete ==="
    exit "$exit_code"
}
trap cleanup EXIT INT TERM

# Check for root/sudo (needed for KVM and disk formatting)
if [[ "$EUID" -ne 0 ]]; then
    echo "This script requires root privileges for KVM and disk setup."
    echo "Please run with: sudo $0 $*"
    exit 1
fi

for cmd in qemu-system-x86_64 mkfs.ext4 dd; do
    if ! command -v "$cmd" >/dev/null 2>&1; then
        echo "Error: Required command not found: $cmd"
        exit 1
    fi
done

if [[ "$AUTO_START_KATANA" -eq 1 ]]; then
    if ! command -v socat >/dev/null 2>&1; then
        echo "Error: Required command not found: socat"
        echo "Install socat or run with --no-start."
        exit 1
    fi
fi

# Verify files exist
echo "Checking TEE boot components..."
for file in "$OVMF_FILE" "$KERNEL_FILE" "$INITRD_FILE"; do
    if [[ ! -f "$file" ]]; then
        echo "Error: Missing $file"
        exit 1
    fi
    echo "  Found: $file ($(ls -lh "$file" | awk '{print $5}'))"
done

# Prepare required ext4 data disk for /dev/sda
echo ""
echo "Preparing VM data disk..."
dd if=/dev/zero of="$DISK_IMAGE" bs=1M count="$DISK_SIZE_MB" status=none
mkfs.ext4 -F -q "$DISK_IMAGE"
echo "  Disk image: $DISK_IMAGE (${DISK_SIZE_MB}MB, ext4)"

echo ""
echo "Starting TEE QEMU VM..."
echo "  OVMF:           $OVMF_FILE"
echo "  Kernel:         $KERNEL_FILE"
echo "  Initrd:         $INITRD_FILE"
echo "  Cmdline:        $KERNEL_CMDLINE"
echo "  Policy:         $GUEST_POLICY"
echo "  vCPUs:          $VCPU_COUNT"
echo "  Memory:         $MEMORY"
echo "  Serial:         $SERIAL_LOG"
echo "  Control socket: $CONTROL_SOCKET"
echo "  RPC:            localhost:$HOST_RPC_PORT -> VM:$KATANA_RPC_PORT"
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
    -device virtio-serial-pci,id=virtio-serial0 \
    -chardev socket,id=katanactl,path="$CONTROL_SOCKET",server=on,wait=off \
    -device virtserialport,chardev=katanactl,name="$CONTROL_PORT_NAME" \
    -device virtio-scsi-pci,id=scsi0 \
    -drive file="$DISK_IMAGE",format=raw,if=none,id=disk0,cache=none \
    -device scsi-hd,drive=disk0,bus=scsi0.0 \
    -netdev user,id=net0,hostfwd=tcp::${HOST_RPC_PORT}-:${KATANA_RPC_PORT} \
    -device virtio-net-pci,disable-legacy=on,iommu_platform=true,netdev=net0,romfile= \
    &

QEMU_PID=$!
echo "QEMU started with PID $QEMU_PID"

# Wait for serial log file to be created
echo ""
echo "Waiting for serial log file..."
while [[ ! -f "$SERIAL_LOG" ]]; do
    if ! kill -0 "$QEMU_PID" 2>/dev/null; then
        echo "Error: QEMU process died before creating serial log"
        show_serial_tail
        exit 1
    fi
    sleep 0.1
done
echo "Serial log file created"

if [[ "$AUTO_START_KATANA" -eq 1 ]]; then
    echo ""
    echo "Waiting for control socket..."
    waited=0
    while [[ ! -S "$CONTROL_SOCKET" ]]; do
        if ! kill -0 "$QEMU_PID" 2>/dev/null; then
            echo "Error: QEMU process died before control socket became ready"
            show_serial_tail
            exit 1
        fi

        sleep 1
        waited=$((waited + 1))
        if [[ "$waited" -ge "$CONTROL_TIMEOUT" ]]; then
            echo "Error: Timeout waiting for control socket: $CONTROL_SOCKET"
            show_serial_tail
            exit 1
        fi
    done
    echo "Control socket ready"

    echo ""
    echo "Sending async Katana start command..."
    START_RESPONSE="$(send_control_command "start $KATANA_ARGS_CSV" || true)"
    if [[ -z "$START_RESPONSE" ]]; then
        echo "Error: No response from guest control channel"
        show_serial_tail
        exit 1
    fi
    echo "  Start response: $START_RESPONSE"

    case "$START_RESPONSE" in
        ok\ started*|err\ already-running*)
            ;;
        *)
            echo "Error: Unexpected start response from guest"
            show_serial_tail
            exit 1
            ;;
    esac

    echo ""
    echo "Waiting for Katana running status..."
    waited=0
    while true; do
        if ! kill -0 "$QEMU_PID" 2>/dev/null; then
            echo "Error: QEMU process died while waiting for Katana"
            show_serial_tail
            exit 1
        fi

        STATUS_RESPONSE="$(send_control_command "status" || true)"
        if [[ "$STATUS_RESPONSE" == running\ * ]]; then
            echo "  Status: $STATUS_RESPONSE"
            break
        fi

        sleep 1
        waited=$((waited + 1))
        if [[ "$waited" -ge "$CONTROL_TIMEOUT" ]]; then
            echo "Error: Timeout waiting for Katana to report running"
            echo "  Last status: ${STATUS_RESPONSE:-<none>}"
            show_serial_tail
            exit 1
        fi
    done
else
    echo ""
    echo "Katana auto-start disabled (--no-start)."
    echo "Use the control socket to send commands manually:"
    echo "  printf 'start $KATANA_ARGS_CSV\n' | socat - UNIX-CONNECT:$CONTROL_SOCKET"
    echo "  printf 'status\n' | socat - UNIX-CONNECT:$CONTROL_SOCKET"
fi

echo ""
echo "=== Following serial output (Ctrl+C to exit) ==="
tail -f "$SERIAL_LOG"
