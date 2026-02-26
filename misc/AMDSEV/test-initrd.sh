#!/bin/bash
# ==============================================================================
# TEST-INITRD.SH - Isolated initrd validation for AMDSEV
# ==============================================================================
#
# Runs focused checks for initrd behavior without requiring the full SEV-SNP
# launch path:
#   1) Static archive/content checks (no VM boot)
#   2) Plain-QEMU boot smoke test with RPC health check (no OVMF/SEV)
#
# Usage:
#   ./test-initrd.sh [OPTIONS]
#
# Options:
#   --output-dir DIR      Boot artifacts directory (default: ./output/qemu)
#   --static-only         Run only static initrd checks
#   --boot-only           Run only boot smoke test
#   --host-rpc-port PORT  Host port for forwarded Katana RPC (default: 15052)
#   --vm-rpc-port PORT    Guest Katana RPC port (default: 5050)
#   --timeout SEC         Boot wait timeout in seconds (default: 90)
#   -h, --help            Show usage
#
# Environment:
#   QEMU_BIN         Optional path to qemu-system-x86_64
#   TEST_DISK_SIZE   Ephemeral test disk size (default: 1G)
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUTPUT_DIR="${SCRIPT_DIR}/output/qemu"
INITRD_FILE=""
KERNEL_FILE=""
RUN_STATIC=1
RUN_BOOT=1
HOST_RPC_PORT=15052
VM_RPC_PORT=5050
BOOT_TIMEOUT=90
TEST_DISK_SIZE="${TEST_DISK_SIZE:-1G}"

TEMP_DIR="$(mktemp -d /tmp/katana-amdsev-initrd-test.XXXXXX)"
EXTRACT_DIR="${TEMP_DIR}/extract"
SERIAL_LOG="${TEMP_DIR}/serial.log"
DISK_IMG="${TEMP_DIR}/test-disk.img"
QEMU_PID=""

usage() {
    cat <<USAGE
Usage: $0 [OPTIONS]

Options:
  --output-dir DIR      Boot artifacts directory (default: ./output/qemu)
  --static-only         Run only static initrd checks
  --boot-only           Run only boot smoke test
  --host-rpc-port PORT  Host port for forwarded Katana RPC (default: 15052)
  --vm-rpc-port PORT    Guest Katana RPC port (default: 5050)
  --timeout SEC         Boot wait timeout in seconds (default: 90)
  -h, --help            Show this help
USAGE
}

log() {
    echo "[test-initrd] $*"
}

warn() {
    echo "[test-initrd] WARN: $*" >&2
}

die() {
    echo "[test-initrd] ERROR: $*" >&2
    exit 1
}

require_tool() {
    local tool="$1"
    command -v "$tool" >/dev/null 2>&1 || die "Required tool not found: $tool"
}

cleanup() {
    local exit_code=$?

    if [ -n "$QEMU_PID" ] && kill -0 "$QEMU_PID" 2>/dev/null; then
        log "Stopping QEMU (PID $QEMU_PID)..."
        kill "$QEMU_PID" 2>/dev/null || true

        for _ in $(seq 1 10); do
            if ! kill -0 "$QEMU_PID" 2>/dev/null; then
                break
            fi
            sleep 0.5
        done

        if kill -0 "$QEMU_PID" 2>/dev/null; then
            warn "QEMU still running, force killing"
            kill -9 "$QEMU_PID" 2>/dev/null || true
        fi

        wait "$QEMU_PID" 2>/dev/null || true
    fi

    rm -rf "$TEMP_DIR"
    exit "$exit_code"
}
trap cleanup EXIT INT TERM

while [[ $# -gt 0 ]]; do
    case "$1" in
        --output-dir)
            OUTPUT_DIR="${2:?Missing value for --output-dir}"
            shift 2
            ;;
        --static-only)
            RUN_STATIC=1
            RUN_BOOT=0
            shift
            ;;
        --boot-only)
            RUN_STATIC=0
            RUN_BOOT=1
            shift
            ;;
        --host-rpc-port)
            HOST_RPC_PORT="${2:?Missing value for --host-rpc-port}"
            shift 2
            ;;
        --vm-rpc-port)
            VM_RPC_PORT="${2:?Missing value for --vm-rpc-port}"
            shift 2
            ;;
        --timeout)
            BOOT_TIMEOUT="${2:?Missing value for --timeout}"
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            die "Unknown argument: $1"
            ;;
    esac
done

INITRD_FILE="${OUTPUT_DIR}/initrd.img"
KERNEL_FILE="${OUTPUT_DIR}/vmlinuz"

assert_extract_path() {
    local rel_path="$1"
    if [ ! -e "${EXTRACT_DIR}/${rel_path}" ]; then
        die "Expected initrd path missing: ${rel_path}"
    fi
}

assert_init_contains() {
    local pattern="$1"
    if ! grep -Fq -- "$pattern" "${EXTRACT_DIR}/init"; then
        die "Expected pattern missing in init script: ${pattern}"
    fi
}

run_static_checks() {
    log "Running static initrd checks"

    require_tool gzip
    require_tool cpio
    require_tool grep

    [ -f "$INITRD_FILE" ] || die "Initrd not found: $INITRD_FILE"

    if ! gzip -t "$INITRD_FILE" 2>/dev/null; then
        die "Initrd is not valid gzip: $INITRD_FILE"
    fi

    mkdir -p "$EXTRACT_DIR"
    (
        cd "$EXTRACT_DIR"
        gzip -dc "$INITRD_FILE" | cpio -id --quiet
    )

    REQUIRED_PATHS=(
        init
        bin/busybox
        bin/katana
        etc/passwd
        etc/group
        bin/sh
        bin/mount
        bin/umount
        bin/ip
        bin/insmod
        bin/poweroff
        bin/sync
    )

    for path in "${REQUIRED_PATHS[@]}"; do
        assert_extract_path "$path"
    done

    [ -x "${EXTRACT_DIR}/init" ] || die "Init script is not executable"
    [ -x "${EXTRACT_DIR}/bin/katana" ] || die "Katana binary in initrd is not executable"

    assert_init_contains "trap shutdown_handler TERM INT"
    assert_init_contains "poweroff -f"
    assert_init_contains "exec 0</dev/console"
    assert_init_contains "if [ -d /sys/class/net/eth0 ]; then"
    assert_init_contains "katana.args="

    if [ ! -e "${EXTRACT_DIR}/lib/modules/tsm.ko" ]; then
        warn "tsm.ko not present in initrd"
    fi
    if [ ! -e "${EXTRACT_DIR}/lib/modules/sev-guest.ko" ]; then
        warn "sev-guest.ko not present in initrd"
    fi

    log "Static initrd checks passed"
}

resolve_qemu_bin() {
    if [ -n "${QEMU_BIN:-}" ]; then
        echo "$QEMU_BIN"
        return 0
    fi

    if command -v qemu-system-x86_64 >/dev/null 2>&1; then
        command -v qemu-system-x86_64
        return 0
    fi

    if [ -x "${OUTPUT_DIR}/bin/qemu-system-x86_64" ]; then
        echo "${OUTPUT_DIR}/bin/qemu-system-x86_64"
        return 0
    fi

    return 1
}

run_boot_smoke_test() {
    local qemu_bin
    local response=""
    local ready=0

    log "Running plain-QEMU boot smoke test"

    [ -f "$KERNEL_FILE" ] || die "Kernel not found: $KERNEL_FILE"
    [ -f "$INITRD_FILE" ] || die "Initrd not found: $INITRD_FILE"

    qemu_bin="$(resolve_qemu_bin)" || die "qemu-system-x86_64 not found (set QEMU_BIN if needed)"

    require_tool curl
    require_tool mkfs.ext4
    require_tool truncate

    truncate -s "$TEST_DISK_SIZE" "$DISK_IMG"
    mkfs.ext4 -q -F "$DISK_IMG"

    KVM_OPTS=()
    if [ -r /dev/kvm ] && [ -w /dev/kvm ]; then
        KVM_OPTS=(-enable-kvm -cpu host)
        log "Using KVM acceleration"
    else
        warn "/dev/kvm not accessible; using software emulation"
        KVM_OPTS=(-cpu max)
    fi

    "$qemu_bin" \
        "${KVM_OPTS[@]}" \
        -m 512M \
        -smp 1 \
        -nographic \
        -serial "file:$SERIAL_LOG" \
        -kernel "$KERNEL_FILE" \
        -initrd "$INITRD_FILE" \
        -append "console=ttyS0 katana.args=--http.addr,0.0.0.0,--http.port,${VM_RPC_PORT},--tee.provider,sev-snp" \
        -device virtio-scsi-pci,id=scsi0 \
        -drive "file=${DISK_IMG},format=raw,if=none,id=disk0,cache=none" \
        -device scsi-hd,drive=disk0,bus=scsi0.0 \
        -netdev "user,id=net0,hostfwd=tcp::${HOST_RPC_PORT}-:${VM_RPC_PORT}" \
        -device virtio-net-pci,netdev=net0 \
        &

    QEMU_PID=$!
    log "QEMU started with PID $QEMU_PID"

    for ((elapsed = 1; elapsed <= BOOT_TIMEOUT; elapsed++)); do
        if ! kill -0 "$QEMU_PID" 2>/dev/null; then
            warn "QEMU exited before RPC became ready"
            if [ -f "$SERIAL_LOG" ]; then
                echo "=== Serial output ===" >&2
                tail -n 200 "$SERIAL_LOG" >&2 || true
            fi
            die "Boot smoke test failed"
        fi

        response="$(curl -s --max-time 2 -X POST \
            -H "Content-Type: application/json" \
            -d '{"jsonrpc":"2.0","method":"starknet_chainId","id":1}' \
            "http://127.0.0.1:${HOST_RPC_PORT}" || true)"

        if echo "$response" | grep -q '"result"'; then
            ready=1
            break
        fi

        if (( elapsed % 5 == 0 )); then
            log "Waiting for Katana RPC... (${elapsed}s/${BOOT_TIMEOUT}s)"
        fi
        sleep 1
    done

    if [ "$ready" -ne 1 ]; then
        warn "Timed out waiting for Katana RPC"
        if [ -f "$SERIAL_LOG" ]; then
            echo "=== Serial output ===" >&2
            tail -n 200 "$SERIAL_LOG" >&2 || true
        fi
        die "Boot smoke test timed out"
    fi

    log "RPC check passed: $response"
    log "Boot smoke test passed"
}

log "Output directory: $OUTPUT_DIR"
log "Modes: static=$RUN_STATIC boot=$RUN_BOOT"

if [ "$RUN_STATIC" -eq 1 ]; then
    run_static_checks
fi

if [ "$RUN_BOOT" -eq 1 ]; then
    run_boot_smoke_test
fi

log "All requested initrd checks passed"
