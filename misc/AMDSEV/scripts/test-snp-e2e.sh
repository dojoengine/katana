#!/bin/bash
# ==============================================================================
# TEST-SNP-E2E.SH - End-to-end release test on real AMD SEV-SNP hardware
# ==============================================================================
#
# Runs ON an SNP-enabled machine, from a checkout of this repo. Used by the
# snp-e2e workflow over SSH, and runnable directly by anyone with SNP
# hardware. Boots the artifacts as a sealed SEV-SNP guest via start-vm.sh
# and asserts the full trust story:
#
#   1. Artifacts match the SHA-256s recorded in build-info.txt.
#   2. The guest boots, Katana starts via the control channel, RPC answers.
#   3. tee_generateQuote returns a hardware attestation report whose
#      MEASUREMENT equals the expected launch measurement and whose policy
#      is the documented 0x30000.
#   4. Reboot reseal: a second boot opens the existing LUKS volume (no
#      reformat) and Katana finds the already-initialized genesis — proving
#      the sealed disk is bound to the measurement and state persists.
#
# Usage (must run as root — start-vm.sh requires it):
#
#   sudo ./scripts/test-snp-e2e.sh                  # latest published release
#   sudo ./scripts/test-snp-e2e.sh --tag TAG        # a specific release
#   sudo ./scripts/test-snp-e2e.sh --boot-dir DIR   # a local build (output/qemu)
#
# Logs land in $WORKDIR/logs on failure (collected by the CI workflow).
# ==============================================================================

set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VM_REPO="${KATANA_TEE_VM_REPO:-dojoengine/katana}"
# Canonical UUID from build-config: makes the quote's measurement directly
# comparable to the published launch measurement.
CANONICAL_LUKS_UUID="00000000-0000-0000-0000-000000000001"
HOST_RPC="http://127.0.0.1:15051"
BOOT_TIMEOUT=360
TAG=""
BOOT_DIR=""
WORKDIR=""
WRAPPER_PID=""

usage() {
    cat <<USAGE >&2
Usage: sudo $0 [--tag TAG | --boot-dir DIR] [--workdir DIR]

  (no arguments)    test the LATEST published release
  --tag TAG         test a specific published release
                    (e.g. tee-vm-v0.1.0+katana-v1.8.0-rc.5)
  --boot-dir DIR    test a LOCAL build: DIR must contain OVMF.fd, vmlinuz,
                    initrd.img and build-info.txt (a build.sh output dir,
                    e.g. output/qemu). Local builds have no recorded
                    LAUNCH_MEASUREMENT; the expected value is computed with
                    snp-digest when available, otherwise the measurement
                    comparison is skipped with a warning.
  --workdir DIR     scratch directory (default: mktemp under /tmp)
USAGE
    exit 1
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --tag)      TAG="${2:?--tag requires a value}"; shift 2 ;;
        --boot-dir) BOOT_DIR="${2:?--boot-dir requires a value}"; shift 2 ;;
        --workdir)  WORKDIR="${2:?--workdir requires a value}"; shift 2 ;;
        -h|--help)  usage ;;
        *) echo "Unknown argument: $1" >&2; usage ;;
    esac
done
[[ -n "$TAG" && -n "$BOOT_DIR" ]] && { echo "ERROR: --tag and --boot-dir are mutually exclusive" >&2; exit 1; }
[[ "$EUID" -eq 0 ]] || { echo "ERROR: must run as root (start-vm.sh requires it)" >&2; exit 1; }
[[ -n "$BOOT_DIR" && ! -f "$BOOT_DIR/build-info.txt" ]] && { echo "ERROR: $BOOT_DIR is not a build output dir (no build-info.txt)" >&2; exit 1; }
[[ -n "$WORKDIR" ]] || WORKDIR="$(mktemp -d /tmp/snp-e2e-local.XXXXXX)"

log()  { echo "[snp-e2e] $*"; }
fail() { echo "[snp-e2e] FAIL: $*" >&2; exit 1; }

DISK="$WORKDIR/data.img"
LOGS="$WORKDIR/logs"

# Find the serial log / control socket paths that start-vm.sh announced in
# its own log.
serial_log_of() {
    awk -F': *' '/^  Serial:/ { print $2; exit }' "$1"
}
control_socket_of() {
    awk -F': *' '/^  Control socket:/ { print $2; exit }' "$1"
}

# Send one control command to a socket and print the first reply line.
control_send() {
    local sock="$1" cmd="$2"
    { printf '%s\n' "$cmd"; sleep 2; } | socat -t 2 -T 4 - UNIX-CONNECT:"$sock" 2>/dev/null | head -n1 | tr -d '\r'
}

# Stop the VM via the guest's graceful `stop` command (flushes the database
# to the sealed disk before poweroff), falling back for releases whose
# initrd predates `stop`: those reply "err unknown-command" and need the
# legacy writeback wait before a hard stop. Deliberately NO wait on the
# graceful path — that makes boot 2 a regression test for shutdown
# durability (stop immediately after writes; state must survive).
stop_vm_graceful() {
    local startlog="$1"
    local sock reply
    sock="$(control_socket_of "$startlog")"
    if [[ -S "$sock" ]]; then
        reply="$(control_send "$sock" "stop" || true)"
        case "$reply" in
            ok\ stopping*)
                log "graceful stop acknowledged"
                for _ in $(seq 1 60); do
                    pgrep -f "[q]emu-system-x86_64.*${DISK}" >/dev/null || break
                    sleep 1
                done
                ;;
            err\ unknown-command*)
                log "guest initrd predates the stop command — legacy 45s writeback wait"
                sleep 45
                ;;
            *)
                log "no usable reply to stop ('${reply:-<none>}') — legacy 45s writeback wait"
                sleep 45
                ;;
        esac
    fi
    stop_vm
}

# Snapshot diagnostics before any teardown (start-vm.sh deletes its serial
# log on exit).
snapshot_logs() {
    mkdir -p "$LOGS"
    cp -f "$WORKDIR"/start*.log "$LOGS/" 2>/dev/null || true
    local s
    for f in "$WORKDIR"/start*.log; do
        [[ -f "$f" ]] || continue
        s="$(serial_log_of "$f")"
        [[ -n "$s" && -f "$s" ]] && cp -f "$s" "$LOGS/serial-$(basename "$f" .log).log"
    done
}

# Kill ONLY processes belonging to this test (identified by our unique disk
# path in their command lines) — this is a shared machine.
stop_vm() {
    if [[ -n "$WRAPPER_PID" ]] && kill -0 "$WRAPPER_PID" 2>/dev/null; then
        kill "$WRAPPER_PID" 2>/dev/null || true
        for _ in $(seq 1 10); do kill -0 "$WRAPPER_PID" 2>/dev/null || break; sleep 1; done
        kill -9 "$WRAPPER_PID" 2>/dev/null || true
    fi
    WRAPPER_PID=""
    pkill -f "[q]emu-system-x86_64.*${DISK}" 2>/dev/null || true
    for _ in $(seq 1 10); do pgrep -f "[q]emu-system-x86_64.*${DISK}" >/dev/null || break; sleep 1; done
    pkill -9 -f "[q]emu-system-x86_64.*${DISK}" 2>/dev/null || true
}

on_exit() {
    local rc=$?
    if [[ $rc -ne 0 ]]; then
        snapshot_logs
        echo "[snp-e2e] diagnostics saved to $LOGS"
    fi
    stop_vm
    exit "$rc"
}
trap on_exit EXIT INT TERM

# ------------------------------------------------------------------------------
# Preflight
# ------------------------------------------------------------------------------
log "Preflight"
[[ -e /dev/sev ]] || fail "/dev/sev not present — not an SNP host?"
[[ "$(cat /sys/module/kvm_amd/parameters/sev_snp 2>/dev/null)" == "Y" ]] || fail "kvm_amd sev_snp not enabled"
for t in qemu-system-x86_64 socat curl python3 mkfs.ext4 dd; do
    command -v "$t" >/dev/null || fail "missing tool: $t"
done

# Clean leftovers from previous runs of THIS test only, then check the port.
# (Matches both the CI workdir /tmp/snp-e2e/… and local /tmp/snp-e2e-local.…)
pkill -f "[q]emu-system-x86_64.*/snp-e2e" 2>/dev/null || true
sleep 2
if curl -s --max-time 2 -o /dev/null "$HOST_RPC"; then
    fail "port 15051 already in use by a foreign process — refusing to continue on a shared machine"
fi

rm -rf "$WORKDIR"
mkdir -p "$WORKDIR" "$LOGS"

# ------------------------------------------------------------------------------
# Boot artifacts: a local build dir, a named release, or the latest release
# ------------------------------------------------------------------------------
if [[ -n "$BOOT_DIR" ]]; then
    log "Using local build: $BOOT_DIR"
    mkdir -p "$WORKDIR/boot"
    for f in OVMF.fd vmlinuz initrd.img build-info.txt; do
        [[ -f "$BOOT_DIR/$f" ]] || fail "local build dir is missing $f"
        cp "$BOOT_DIR/$f" "$WORKDIR/boot/$f"
    done
    UNDER_TEST="local build $BOOT_DIR"
else
    if [[ -z "$TAG" ]]; then
        # Only tee-vm-v* tags are TEE-VM releases; the repo also publishes
        # katana's own vX.Y.Z releases, which carry no VM image.
        log "Resolving latest TEE-VM release (tee-vm-v*)"
        TAG="$(curl -fsSL "https://api.github.com/repos/${VM_REPO}/releases?per_page=30" \
            | python3 -c 'import json,sys; rs=[r["tag_name"] for r in json.load(sys.stdin) if r["tag_name"].startswith("tee-vm-v")]; print(rs[0] if rs else "")')"
        [[ -n "$TAG" ]] || fail "no published tee-vm-v* releases found in $VM_REPO"
    fi
    log "Downloading release $TAG"
    # The tag/asset name carries a '+' (SemVer build metadata); percent-encode
    # it so curl sends a literal '+' in the URL path rather than a space.
    TAG_URL="${TAG//+/%2B}"
    curl -fsSL -o "$WORKDIR/release.tar.gz" \
        "https://github.com/${VM_REPO}/releases/download/${TAG_URL}/katana-tee-vm-${TAG_URL}.tar.gz" \
        || fail "could not download release tarball for $TAG"
    mkdir -p "$WORKDIR/boot"
    tar xzf "$WORKDIR/release.tar.gz" -C "$WORKDIR/boot"
    UNDER_TEST="$TAG"
fi

BUILD_INFO="$WORKDIR/boot/build-info.txt"
[[ -f "$BUILD_INFO" ]] || fail "no build-info.txt among the boot artifacts"
info_get() { awk -F= -v k="$1" '$1 == k { sub(/^[^=]*=/, ""); print; exit }' "$BUILD_INFO"; }

log "Verifying artifact checksums"
for pair in "OVMF.fd:OVMF_SHA256" "vmlinuz:KERNEL_SHA256" "initrd.img:INITRD_SHA256"; do
    f="${pair%%:*}"; k="${pair##*:}"
    actual="$(sha256sum "$WORKDIR/boot/$f" | awk '{print $1}')"
    expected="$(info_get "$k")"
    [[ "$actual" == "$expected" ]] || fail "$f sha256 mismatch (got $actual, recorded $expected)"
done
log "Checksums OK"

# Expected measurement: releases record it in build-info (bound to the
# canonical LUKS UUID). Local builds don't — compute it with snp-digest when
# available, otherwise skip the comparison loudly.
MEASUREMENT_CHECK=1
EXPECTED_MEASUREMENT="$(info_get LAUNCH_MEASUREMENT)"
if [[ -n "$EXPECTED_MEASUREMENT" ]]; then
    RECORDED_LUKS_UUID="$(info_get LUKS_UUID)"
    [[ "$RECORDED_LUKS_UUID" == "$CANONICAL_LUKS_UUID" ]] \
        || fail "recorded measurement is bound to LUKS_UUID '$RECORDED_LUKS_UUID', expected canonical '$CANONICAL_LUKS_UUID'"
else
    SNP_DIGEST=""
    if command -v snp-digest >/dev/null 2>&1; then
        SNP_DIGEST="$(command -v snp-digest)"
    else
        SNP_DIGEST="$(find "$REPO_DIR/snp-tools/target" -type f -name snp-digest -perm -u+x 2>/dev/null | head -n1)"
    fi
    if [[ -n "$SNP_DIGEST" ]]; then
        # shellcheck source=scripts/sealed-cmdline.sh
        . "$REPO_DIR/scripts/sealed-cmdline.sh"
        EXPECTED_MEASUREMENT="$("$SNP_DIGEST" \
            --ovmf="$WORKDIR/boot/OVMF.fd" \
            --kernel="$WORKDIR/boot/vmlinuz" \
            --initrd="$WORKDIR/boot/initrd.img" \
            --append="$(build_sealed_cmdline "$CANONICAL_LUKS_UUID")" \
            --vcpus=1 --cpu=epyc-v4 --vmm=qemu --guest-features=0x1)"
        log "Computed expected measurement with snp-digest"
    else
        MEASUREMENT_CHECK=0
        log "WARNING: no LAUNCH_MEASUREMENT recorded and snp-digest not found —"
        log "WARNING: the measurement comparison will be SKIPPED. Build snp-tools"
        log "WARNING: (cd snp-tools && cargo build --release) for the full check."
    fi
fi
[[ "$MEASUREMENT_CHECK" -eq 1 ]] && log "Expected measurement: $EXPECTED_MEASUREMENT"

dd if=/dev/zero of="$DISK" bs=1M count=1024 status=none

# ------------------------------------------------------------------------------
# Boot helpers
# ------------------------------------------------------------------------------
launch_vm() {
    local startlog="$1"
    ( cd "$REPO_DIR" && nohup ./start-vm.sh \
        --ovmf "$WORKDIR/boot/OVMF.fd" \
        --kernel "$WORKDIR/boot/vmlinuz" \
        --initrd "$WORKDIR/boot/initrd.img" \
        --data-disk "$DISK" \
        --sealed \
        --luks-uuid "$CANONICAL_LUKS_UUID" \
        > "$startlog" 2>&1 & echo $! > "$WORKDIR/wrapper.pid" )
    WRAPPER_PID="$(cat "$WORKDIR/wrapper.pid")"
}

wait_running() {
    local startlog="$1"
    local waited=0
    while true; do
        grep -q "Status: running pid=" "$startlog" 2>/dev/null && return 0
        if grep -qE "^Error:|Timeout" "$startlog" 2>/dev/null; then
            tail -30 "$startlog" >&2
            fail "start-vm.sh reported an error (see $startlog)"
        fi
        kill -0 "$WRAPPER_PID" 2>/dev/null || { tail -30 "$startlog" >&2; fail "start-vm.sh exited prematurely"; }
        sleep 5; waited=$((waited + 5))
        [[ "$waited" -lt "$BOOT_TIMEOUT" ]] || { tail -30 "$startlog" >&2; fail "timed out waiting for Katana (${BOOT_TIMEOUT}s)"; }
    done
}

rpc() {
    curl -s --max-time 30 -X POST -H "Content-Type: application/json" -d "$1" "$HOST_RPC"
}

# Returns "measurement policy" parsed from a tee_generateQuote response.
# The response is passed via the environment and the Python source via a
# quoted heredoc — no shell escaping inside the Python at all (a previous
# version used escaped quotes inside a single-quoted -c string, which reach
# Python as literal backslashes and are a syntax error).
quote_fields() {
    QUOTE_JSON="$(rpc '{"jsonrpc":"2.0","id":1,"method":"tee_generateQuote","params":[null,0]}')" \
    python3 <<'PYEOF'
import json, os, sys

r = json.loads(os.environ["QUOTE_JSON"])
if "error" in r:
    sys.exit("tee_generateQuote error: %s" % r["error"])
q = bytes.fromhex(r["result"]["quote"].removeprefix("0x"))
print(q[0x90:0x90+48].hex(), hex(int.from_bytes(q[8:16], "little")))
PYEOF
}

# ------------------------------------------------------------------------------
# Boot 1: fresh disk — format, attest, compare measurement
# ------------------------------------------------------------------------------
log "Boot 1: fresh sealed disk"
launch_vm "$WORKDIR/start1.log"
wait_running "$WORKDIR/start1.log"
log "Katana running"

CHAIN_ID="$(rpc '{"jsonrpc":"2.0","method":"starknet_chainId","id":1}' | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"])')"
[[ "$CHAIN_ID" == "0x4b4154414e41" ]] || fail "unexpected chainId: $CHAIN_ID"
log "RPC OK (chainId $CHAIN_ID)"

read -r MEASUREMENT1 POLICY1 <<< "$(quote_fields)"
log "quote 1: measurement=$MEASUREMENT1 policy=$POLICY1"
[[ "$POLICY1" == "0x30000" ]] || fail "unexpected guest policy: $POLICY1"
if [[ "$MEASUREMENT_CHECK" -eq 1 ]]; then
    [[ "$MEASUREMENT1" == "$EXPECTED_MEASUREMENT" ]] \
        || fail "measurement does not match expected:
  quote:    $MEASUREMENT1
  expected: $EXPECTED_MEASUREMENT"
    log "measurement matches expected value"
else
    log "measurement comparison SKIPPED (no expected value available)"
fi

# ------------------------------------------------------------------------------
# Boot 2: reboot reseal + state persistence
# ------------------------------------------------------------------------------
# Graceful stop immediately after the writes: the guest's `stop` command
# flushes everything to the sealed disk before poweroff, so boot 2 doubles
# as a regression test for shutdown durability. (Releases predating `stop`
# get the legacy 45s writeback wait inside stop_vm_graceful.)
log "Boot 2: graceful stop, then reboot with existing sealed disk"
stop_vm_graceful "$WORKDIR/start1.log"
launch_vm "$WORKDIR/start2.log"
wait_running "$WORKDIR/start2.log"

SERIAL2="$(serial_log_of "$WORKDIR/start2.log")"
[[ -n "$SERIAL2" && -f "$SERIAL2" ]] || fail "cannot locate serial log of boot 2"
grep -aq "No LUKS header" "$SERIAL2" \
    && fail "boot 2 reformatted the disk — sealed state was lost (derived key mismatch?)"
grep -aq "Genesis has already been initialized" "$SERIAL2" \
    || fail "boot 2 did not find the existing genesis — state did not persist"

read -r MEASUREMENT2 _ <<< "$(quote_fields)"
[[ "$MEASUREMENT2" == "$MEASUREMENT1" ]] || fail "boot 2 measurement drifted: $MEASUREMENT2 (boot 1: $MEASUREMENT1)"
log "reseal OK: existing LUKS opened, genesis persisted, measurement stable"

# ------------------------------------------------------------------------------
log "Tearing down"
stop_vm
rm -rf "$WORKDIR"
trap - EXIT
echo ""
echo "=========================================="
echo "SNP E2E PASS: $UNDER_TEST"
echo "  measurement: $MEASUREMENT1"
[[ "$MEASUREMENT_CHECK" -eq 1 ]] || echo "  (measurement comparison was skipped — no expected value)"
echo "=========================================="
