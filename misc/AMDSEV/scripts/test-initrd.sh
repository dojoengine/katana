#!/bin/bash
# ==============================================================================
# TEST-INITRD.SH - Isolated initrd validation for AMDSEV
# ==============================================================================
#
# Runs a focused initrd boot smoke test without requiring the full SEV-SNP
# launch path. Uses plain QEMU (no OVMF/SEV), delivers Katana's CLI args via
# fw_cfg (opt/org.katana/args, same mechanism as start-vm.sh), packs a
# synthetic chain config dir into a read-only virtio-blk ext2 disk, starts
# Katana through the async control channel, and validates RPC readiness.
# Along the way it asserts the control-channel protocol contract: a
# `start` with a payload is rejected (config comes from fw_cfg), unknown
# commands error, a rejected start does not launch Katana, and a duplicate
# start is refused.
#
# Also asserts a tight time budget on the "Chain config disk mounted" log
# line — a regression test guarding against reverting chain delivery back to
# fw_cfg (under SEV-SNP that would wedge the guest for >10 minutes; even on
# plain KVM the slow path takes minutes). See the rationale comment in
# scripts/build-initrd.sh ("Install QEMU fw_cfg Kernel Module").
#
# Usage:
#   ./test-initrd.sh [OPTIONS]
#
# Options:
#   --output-dir DIR      Boot artifacts directory (default: ./output/qemu)
#   --host-rpc-port PORT  Host port for forwarded Katana RPC (default: 15052)
#   --vm-rpc-port PORT    Guest Katana RPC port (default: 5050)
#   --timeout SEC         Boot wait timeout in seconds (default: 90)
#   -h, --help            Show usage
#
# Environment:
#   QEMU_BIN                    Optional path to qemu-system-x86_64
#   TEST_DISK_SIZE              Ephemeral test disk size (default: 1G)
#   CHAIN_CONFIG_GENESIS_SIZE   Synthetic genesis size for chain-disk regression
#                               test (default: 18M, matching production)
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUTPUT_DIR="${SCRIPT_DIR}/output/qemu"
INITRD_FILE=""
KERNEL_FILE=""
HOST_RPC_PORT=15052
VM_RPC_PORT=5050
BOOT_TIMEOUT=90
TEST_DISK_SIZE="${TEST_DISK_SIZE:-1G}"
# Size of the synthetic genesis.json used by the chain-disk regression check.
# Matches production CARTRIDGE_SEPOLIA's genesis size (~18 MB) so the test
# would catch a regression to fw_cfg-based chain delivery, where this would
# stall the guest for >10 minutes due to the quadratic sysfs read pattern
# in the upstream qemu_fw_cfg driver. See the rationale comment in
# scripts/build-initrd.sh's "Install QEMU fw_cfg + virtio_blk" section.
CHAIN_CONFIG_GENESIS_SIZE="${CHAIN_CONFIG_GENESIS_SIZE:-18M}"
# Time budget for the guest's "Chain config disk mounted" log line to appear
# after QEMU starts. virtio-blk + ext2 mount takes <1s in practice; this
# bound catches a regression long before BOOT_TIMEOUT would.
CHAIN_MOUNT_TIMEOUT=30

TEMP_DIR="$(mktemp -d /tmp/katana-amdsev-initrd-test.XXXXXX)"
SERIAL_LOG="${TEMP_DIR}/serial.log"
DISK_IMG="${TEMP_DIR}/test-disk.img"
CONTROL_SOCKET="${TEMP_DIR}/katana-control.sock"
KATANA_ARGS_FILE="${TEMP_DIR}/katana-args.txt"
CHAIN_DIR="${TEMP_DIR}/chain-config"
CHAIN_IMG="${TEMP_DIR}/chain.img"
QEMU_PID=""

usage() {
    cat <<USAGE
Usage: $0 [OPTIONS]

Options:
  --output-dir DIR      Boot artifacts directory (default: ./output/qemu)
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

# Build an ext2 image at $CHAIN_IMG sized to match production
# ($CHAIN_CONFIG_GENESIS_SIZE, default 18M) with synthetic content. We do NOT
# put a katana-parseable chain spec here because:
#   - this test runs against whatever katana version the workflow downloads,
#     and the chain-spec TOML/genesis schema has shifted across katana minor
#     releases (e.g. proof_kind landed post-v1.7);
#   - the regression we're guarding against ("did chain delivery slip back to
#     the slow fw_cfg path?") is signaled by the marker text in the guest
#     init's mount log, not by katana parsing the spec successfully.
# mount_chain_disk in build-initrd.sh treats a chain dir without config.toml
# as "mount succeeded, no --chain to katana", which gives us the marker
# without coupling this test to a specific katana version.
#
# The file is filled with /dev/urandom + conv=fsync to put incompressible
# bytes on disk — avoids any sparse-file optimization that would mask a
# regression to a transport whose perf scales with the *populated* byte count.
build_chain_config_disk() {
    require_tool mkfs.ext2
    require_tool truncate
    require_tool dd

    mkdir -p "$CHAIN_DIR"
    dd if=/dev/urandom of="$CHAIN_DIR/padding.bin" \
        bs=1M count="${CHAIN_CONFIG_GENESIS_SIZE%M}" \
        status=none conv=fsync

    chain_dir_mb=$(du -sm "$CHAIN_DIR" | awk '{print $1}')
    img_mb=$(( chain_dir_mb * 2 + 16 ))
    truncate -s "${img_mb}M" "$CHAIN_IMG"
    mkfs.ext2 -q -F -d "$CHAIN_DIR" -L katana-chain -E no_copy_xattrs "$CHAIN_IMG"
    log "chain disk: ${chain_dir_mb} MB content packed into ${img_mb} MB ext2 image"
}

# Watch the serial log for "Chain config disk mounted at" within
# $CHAIN_MOUNT_TIMEOUT seconds of QEMU starting. The whole point of this
# test is to detect a regression to the slow fw_cfg chain delivery path —
# which under SEV-SNP wedges the guest for >10 minutes, and on plain KVM
# (this test) still takes minutes. virtio-blk should print this line in <1s.
wait_for_chain_mount() {
    local started_at marker="Chain config disk mounted at"
    started_at=$(date +%s)
    log "Watching serial log for: '$marker' (budget: ${CHAIN_MOUNT_TIMEOUT}s)"
    while true; do
        assert_qemu_running "Chain disk mount regression check failed"
        if grep -qF "$marker" "$SERIAL_LOG" 2>/dev/null; then
            local elapsed=$(( $(date +%s) - started_at ))
            log "Chain disk mount: PASS (${elapsed}s)"
            return 0
        fi
        if (( $(date +%s) - started_at >= CHAIN_MOUNT_TIMEOUT )); then
            warn "Chain disk mount regression detected: '$marker' not seen in ${CHAIN_MOUNT_TIMEOUT}s"
            warn "This likely means chain config delivery regressed to a slow path"
            warn "(was the fw_cfg-vs-virtio-blk switch in build-initrd.sh reverted?)"
            print_serial_output
            die "Chain disk mount timed out — see scripts/build-initrd.sh comment block"
        fi
        sleep 1
    done
}

print_serial_output() {
    if [ -f "$SERIAL_LOG" ]; then
        echo "=== Serial output ===" >&2
        tail -n 200 "$SERIAL_LOG" >&2 || true
    fi
}

assert_qemu_running() {
    local message="$1"
    if ! kill -0 "$QEMU_PID" 2>/dev/null; then
        warn "QEMU exited unexpectedly"
        print_serial_output
        die "$message"
    fi
}

send_control_command() {
    local cmd="$1"
    # Keep stdin open for a window after the command (same workaround as
    # send_control_command in start-vm.sh): socat closes the write side of
    # the unix socket as soon as stdin EOFs, QEMU treats that as a full
    # chardev disconnect, and the guest's reply written to the virtio-serial
    # port is dropped before it can flow back. The sleep gives the guest's
    # read -> handle -> respond round-trip time to land.
    { printf '%s\n' "$cmd"; sleep 2; } | socat -t 2 -T 4 - UNIX-CONNECT:"$CONTROL_SOCKET" 2>/dev/null | head -n 1 | tr -d '\r'
}

wait_for_control_channel() {
    local response=""

    for ((elapsed = 1; elapsed <= BOOT_TIMEOUT; elapsed++)); do
        assert_qemu_running "Boot smoke test failed"

        if [ -S "$CONTROL_SOCKET" ]; then
            response="$(send_control_command status || true)"
            case "$response" in
                running\ *|stopped\ *)
                    log "Control channel ready: $response"
                    return 0
                    ;;
            esac
        fi

        if (( elapsed % 5 == 0 )); then
            log "Waiting for control channel... (${elapsed}s/${BOOT_TIMEOUT}s)"
        fi
        sleep 1
    done

    warn "Timed out waiting for control channel"
    print_serial_output
    die "Boot smoke test timed out"
}

# Send a control command and require a response with the given prefix.
# The guest's reply can be dropped if socat tears the socket down before it
# lands (the same race the start/status polls tolerate by retrying), so an
# empty reply is retried; a non-empty wrong reply is a deterministic protocol
# violation and fails immediately.
expect_control_response() {
    local cmd="$1"
    local expected_prefix="$2"
    local label="$3"
    local response=""

    for _ in 1 2 3 4 5; do
        response="$(send_control_command "$cmd" || true)"
        case "$response" in
            ${expected_prefix}*)
                log "  ${label}: ${response}"
                return 0
                ;;
        esac
        [ -n "$response" ] && break
        sleep 1
    done

    warn "Unexpected response to ${label}: '${response:-<none>}' (expected '${expected_prefix} ...')"
    print_serial_output
    die "Control protocol check failed: ${label}"
}

# Protocol assertions that must run while Katana is NOT yet running. The
# init's start handler checks already-running before it checks for a payload,
# so the payload rejection is only observable pre-start.
verify_control_protocol_prestart() {
    log "Verifying control-channel protocol (pre-start)"
    # Launch config comes from fw_cfg: the old `start <csv-args>` protocol
    # must be rejected loudly, not have its args silently dropped...
    expect_control_response "start --http.port,9999" "err start-takes-no-args" "start with payload"
    # ...and unknown commands must error rather than being ignored.
    expect_control_response "bogus-command" "err unknown-command" "unknown command"
    # The rejected start must not have launched Katana.
    expect_control_response "status" "stopped" "status after rejected start"
}

verify_duplicate_start_rejected() {
    log "Verifying duplicate start is refused"
    expect_control_response "start" "err already-running" "duplicate start"
}

start_katana_via_control_channel() {
    # Bare `start` — CLI args were delivered via fw_cfg at QEMU launch.
    local start_cmd="start"
    local response=""

    for ((elapsed = 1; elapsed <= BOOT_TIMEOUT; elapsed++)); do
        assert_qemu_running "Boot smoke test failed"

        response="$(send_control_command "$start_cmd" || true)"
        case "$response" in
            ok\ started\ *|err\ already-running\ *)
                log "Katana start acknowledged: $response"
                return 0
                ;;
        esac

        if (( elapsed % 5 == 0 )); then
            log "Waiting for Katana start acknowledgement... (${elapsed}s/${BOOT_TIMEOUT}s)"
        fi
        sleep 1
    done

    warn "Timed out waiting for Katana start acknowledgement"
    print_serial_output
    die "Boot smoke test timed out"
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
    require_tool socat

    truncate -s "$TEST_DISK_SIZE" "$DISK_IMG"
    mkfs.ext4 -q -F "$DISK_IMG"

    # Build a synthetic chain config disk so we exercise the virtio-blk path
    # and can assert it mounts within the time budget (see wait_for_chain_mount).
    build_chain_config_disk

    # Katana CLI args, one per line, delivered via fw_cfg (unmeasured —
    # same default invocation as start-vm.sh, including --tee sev-snp).
    # Requires a TEE-capable katana (v1.8.0-rc.1+); the CI workflow pins
    # KATANA_TEST_VERSION accordingly.
    #
    # `--http.cors-origins *` is included as a regression test for the
    # strip_reserved_args glob-expansion bug in build-initrd.sh: without
    # `set -f` around the unquoted `for tok in $*` loop, the guest sh would
    # pathname-expand the `*` to the rootfs's top-level dirs (bin dev etc
    # init lib lib64 …) before reaching katana — katana would then see
    # `--http.cors-origins bin dev etc init …`, reject `dev` as an unknown
    # flag, exit, and the RPC liveness check below would time out.
    printf '%s\n' \
        "--http.addr" "0.0.0.0" \
        "--http.port" "${VM_RPC_PORT}" \
        "--http.cors-origins" "*" \
        "--tee" "sev-snp" \
        > "$KATANA_ARGS_FILE"

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
        -append "console=ttyS0" \
        -device virtio-serial-pci,id=virtio-serial0 \
        -chardev "socket,id=katanactl,path=${CONTROL_SOCKET},server=on,wait=off" \
        -device virtserialport,chardev=katanactl,name=org.katana.control.0 \
        -fw_cfg "name=opt/org.katana/args,file=${KATANA_ARGS_FILE}" \
        -drive "file=${CHAIN_IMG},format=raw,if=none,id=chaincfg,readonly=on" \
        -device virtio-blk-pci,drive=chaincfg,serial=katana-chain \
        -device virtio-scsi-pci,id=scsi0 \
        -drive "file=${DISK_IMG},format=raw,if=none,id=disk0,cache=none" \
        -device scsi-hd,drive=disk0,bus=scsi0.0 \
        -netdev "user,id=net0,hostfwd=tcp::${HOST_RPC_PORT}-:${VM_RPC_PORT}" \
        -device virtio-net-pci,netdev=net0 \
        &

    QEMU_PID=$!
    log "QEMU started with PID $QEMU_PID"

    # Regression check: the guest must mount the virtio-blk chain disk
    # within CHAIN_MOUNT_TIMEOUT. Catches a revert to fw_cfg-based chain
    # delivery, which under SEV-SNP would wedge here for >10 minutes.
    wait_for_chain_mount

    wait_for_control_channel
    verify_control_protocol_prestart
    start_katana_via_control_channel
    verify_duplicate_start_rejected

    for ((elapsed = 1; elapsed <= BOOT_TIMEOUT; elapsed++)); do
        assert_qemu_running "Boot smoke test failed"

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
        print_serial_output
        die "Boot smoke test timed out"
    fi

    log "RPC check passed: $response"

    # Graceful stop: the guest must acknowledge, run its teardown (visible
    # in the serial log), and power off — which makes QEMU exit. The ack
    # can be lost in the shutdown race (the guest closes the channel right
    # after responding), so the authoritative signal is QEMU exiting with
    # teardown evidence in the serial log.
    log "Verifying graceful stop"
    response="$(send_control_command "stop" || true)"
    case "$response" in
        ok\ stopping*) log "  stop acknowledged: $response" ;;
        *)             log "  stop reply: '${response:-<none>}' (ack can be lost in shutdown race)" ;;
    esac
    for ((elapsed = 1; elapsed <= 60; elapsed++)); do
        kill -0 "$QEMU_PID" 2>/dev/null || break
        sleep 1
    done
    if kill -0 "$QEMU_PID" 2>/dev/null; then
        warn "Guest did not power off within 60s of stop"
        print_serial_output
        die "Graceful stop failed"
    fi
    if ! grep -aq "Teardown: poweroff" "$SERIAL_LOG"; then
        print_serial_output
        die "Guest exited without running teardown (no 'Teardown: poweroff' in serial log)"
    fi
    log "Graceful stop OK: teardown ran, guest powered off"

    log "Boot smoke test passed"
}

log "Output directory: $OUTPUT_DIR"
run_boot_smoke_test

log "All requested initrd checks passed"
