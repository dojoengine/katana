#!/usr/bin/env bash
# test-db-forward-compat.sh
#
# Regression test: a NEWER TEE-VM release must be able to open the database a
# PREVIOUS TEE-VM release wrote.
#
# Since unsealed storage is the default (see docs/amdsev.md), the TEE VM's data
# disk is an ordinary katana MDBX database on plain ext4 — the VM neither
# encrypts nor reshapes it. So "database forward-compatibility across TEE-VM
# releases" reduces to the katana-db property "a newer katana opens an older
# katana's on-disk database", which this script exercises with two binaries on
# the host — no QEMU, no SEV-SNP hardware.
#
# Flow:
#   1. Fetch the katana bundled in the latest published TEE-VM release
#      (katana-tee-vm-<tag>.tar.gz). This is the "previous release".
#   2. With that OLD binary, create a persistent DB and mine a few blocks.
#   3. With the CURRENT binary (built from the ref under test), open the SAME
#      --db-dir with --db.auto-migrate and assert the chain state survived:
#      same chain id, block height not reset, and the old tip block is present.
#
# A failure here means a change in the tree under test broke the ability of a
# future release to read an existing TEE-VM disk (e.g. a db-format bump without
# a migration, or MIN_OPENABLE_DB_VERSION rising above what shipped releases
# wrote). It is the release-cadence counterpart to the `db-compat-test` crate,
# which pins compatibility against a checked-in fixture.
#
# Usage:
#   KATANA_BIN=/path/to/current/katana ./test-db-forward-compat.sh [options]
#
# Options:
#   --current PATH      Current katana binary (default: $KATANA_BIN).
#   --prev-tag TAG      Baseline TEE-VM release tag (default: latest katana-v*).
#   --prev-katana PATH  Use this baseline binary directly; skip the download.
#   --workdir DIR       Scratch dir (default: a fresh mktemp, removed on exit).
#   --blocks N          Blocks to mine with the baseline binary (default: 5).
#
# Env:
#   KATANA_TEE_VM_REPO  GitHub repo publishing the releases (default dojoengine/katana).
#   GH_TOKEN / GITHUB_TOKEN  Optional; used for the releases API to avoid rate limits.

set -euo pipefail

VM_REPO="${KATANA_TEE_VM_REPO:-dojoengine/katana}"
CURRENT_KATANA="${KATANA_BIN:-}"
PREV_TAG=""
PREV_KATANA=""
WORKDIR=""
BLOCKS=5

log()  { printf '[db-fwd-compat] %s\n' "$*"; }
fail() { printf '[db-fwd-compat] FAIL: %s\n' "$*" >&2; exit 1; }
skip() { printf '[db-fwd-compat] SKIP: %s\n' "$*" >&2; exit 0; }

usage() { sed -n '2,46p' "$0"; }

while [[ $# -gt 0 ]]; do
    case "$1" in
        --current)     CURRENT_KATANA="${2:?--current requires a value}"; shift 2 ;;
        --prev-tag)    PREV_TAG="${2:?--prev-tag requires a value}"; shift 2 ;;
        --prev-katana) PREV_KATANA="${2:?--prev-katana requires a value}"; shift 2 ;;
        --workdir)     WORKDIR="${2:?--workdir requires a value}"; shift 2 ;;
        --blocks)      BLOCKS="${2:?--blocks requires a value}"; shift 2 ;;
        -h|--help)     usage; exit 0 ;;
        *)             echo "Error: unknown argument: $1" >&2; usage; exit 1 ;;
    esac
done

[[ -n "$CURRENT_KATANA" ]] || fail "current katana not set (pass --current or set KATANA_BIN)"
[[ -x "$CURRENT_KATANA" ]] || fail "current katana not executable: $CURRENT_KATANA"
command -v python3 >/dev/null 2>&1 || fail "python3 is required"
command -v curl    >/dev/null 2>&1 || fail "curl is required"

CREATED_WORKDIR=0
if [[ -z "$WORKDIR" ]]; then
    WORKDIR="$(mktemp -d)"
    CREATED_WORKDIR=1
fi

OLD_PID=""
NEW_PID=""
cleanup() {
    [[ -n "$OLD_PID" ]] && kill -KILL "$OLD_PID" 2>/dev/null || true
    [[ -n "$NEW_PID" ]] && kill -KILL "$NEW_PID" 2>/dev/null || true
    [[ "$CREATED_WORKDIR" -eq 1 ]] && rm -rf "$WORKDIR" || true
}
trap cleanup EXIT

# ---- RPC helpers ------------------------------------------------------------

# rpc PORT METHOD [PARAMS_JSON] — raw JSON-RPC call, prints the response body.
rpc() {
    local port="$1" method="$2" params="${3:-[]}"
    curl -fsS -m 15 -H 'content-type: application/json' \
        -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"${method}\",\"params\":${params}}" \
        "http://127.0.0.1:${port}"
}

# rpc_result PORT METHOD [PARAMS_JSON] — prints the `.result`, fails on `.error`.
rpc_result() {
    local out
    out="$(rpc "$@")" || return 1
    printf '%s' "$out" | python3 -c '
import json, sys
d = json.load(sys.stdin)
if "error" in d:
    sys.stderr.write("rpc error: %s\n" % json.dumps(d["error"]))
    sys.exit(1)
r = d.get("result")
print(r if not isinstance(r, (dict, list)) else json.dumps(r))
'
}

# wait_rpc PORT PID — block until the node answers, or its process dies/timeout.
wait_rpc() {
    local port="$1" pid="$2" waited=0
    while (( waited < 90 )); do
        kill -0 "$pid" 2>/dev/null || return 1
        if rpc "$port" starknet_chainId >/dev/null 2>&1; then return 0; fi
        sleep 1; waited=$((waited + 1))
    done
    return 1
}

# stop_katana PID — graceful SIGTERM so the DB flushes, then reap.
stop_katana() {
    local pid="$1" waited=0
    kill -TERM "$pid" 2>/dev/null || true
    while kill -0 "$pid" 2>/dev/null; do
        if (( waited >= 30 )); then kill -KILL "$pid" 2>/dev/null || true; break; fi
        sleep 1; waited=$((waited + 1))
    done
    wait "$pid" 2>/dev/null || true
}

# read_db_version DB_DIR — the 4-byte big-endian db.version, or "none".
read_db_version() {
    local f="$1/db.version"
    [[ -f "$f" ]] || { echo "none"; return; }
    python3 -c 'import sys, struct; print(struct.unpack(">I", open(sys.argv[1], "rb").read(4))[0])' "$f"
}

# ---- Resolve the baseline (previous-release) katana -------------------------

if [[ -z "$PREV_KATANA" ]]; then
    if [[ -z "$PREV_TAG" ]]; then
        log "Resolving latest TEE-VM release (katana-v*) from ${VM_REPO}"
        auth=()
        [[ -n "${GH_TOKEN:-${GITHUB_TOKEN:-}}" ]] && auth=(-H "authorization: Bearer ${GH_TOKEN:-$GITHUB_TOKEN}")
        PREV_TAG="$(curl -fsSL "${auth[@]}" "https://api.github.com/repos/${VM_REPO}/releases?per_page=30" \
            | python3 -c 'import json,sys; rs=[r["tag_name"] for r in json.load(sys.stdin) if r["tag_name"].startswith("katana-v")]; print(rs[0] if rs else "")')" \
            || fail "could not query releases from ${VM_REPO}"
        [[ -n "$PREV_TAG" ]] || skip "no published katana-v* TEE-VM releases in ${VM_REPO} yet — nothing to test forward-compat against"
    fi

    log "Baseline TEE-VM release: ${PREV_TAG}"
    tarball="${WORKDIR}/katana-tee-vm-${PREV_TAG}.tar.gz"
    curl -fsSL -o "$tarball" \
        "https://github.com/${VM_REPO}/releases/download/${PREV_TAG}/katana-tee-vm-${PREV_TAG}.tar.gz" \
        || fail "could not download katana-tee-vm-${PREV_TAG}.tar.gz from ${VM_REPO}"

    mkdir -p "${WORKDIR}/baseline"
    tar -xzf "$tarball" -C "${WORKDIR}/baseline" || fail "could not extract ${tarball}"
    PREV_KATANA="$(find "${WORKDIR}/baseline" -maxdepth 3 -type f -name katana | head -n 1)"
    [[ -n "$PREV_KATANA" ]] || fail "no 'katana' binary inside katana-tee-vm-${PREV_TAG}.tar.gz"
    chmod +x "$PREV_KATANA"
fi

log "Baseline katana: $("$PREV_KATANA" --version 2>/dev/null | head -n 1) ($PREV_KATANA)"
log "Current katana:  $("$CURRENT_KATANA" --version 2>/dev/null | head -n 1) ($CURRENT_KATANA)"

DB="${WORKDIR}/db"
PORT=5050

# ---- Phase 1: write a database with the baseline binary ---------------------

log "Phase 1: baseline binary writes a database and mines ${BLOCKS} blocks"
"$PREV_KATANA" --dev --db-dir "$DB" --http.addr 127.0.0.1 --http.port "$PORT" \
    > "${WORKDIR}/katana-baseline.log" 2>&1 &
OLD_PID=$!
wait_rpc "$PORT" "$OLD_PID" || { tail -n 40 "${WORKDIR}/katana-baseline.log" >&2; fail "baseline katana did not become ready"; }

OLD_CHAINID="$(rpc_result "$PORT" starknet_chainId)" || fail "baseline: starknet_chainId failed"
for i in $(seq 1 "$BLOCKS"); do
    rpc "$PORT" dev_generateBlock >/dev/null || fail "baseline: dev_generateBlock failed at block ${i}"
done
OLD_BLOCK="$(rpc_result "$PORT" starknet_blockNumber)" || fail "baseline: starknet_blockNumber failed"
[[ "$OLD_BLOCK" -ge "$BLOCKS" ]] || fail "baseline mined ${OLD_BLOCK} blocks, expected >= ${BLOCKS}"

stop_katana "$OLD_PID"; OLD_PID=""
OLD_DBVER="$(read_db_version "$DB")"
log "baseline wrote chain ${OLD_CHAINID} at block ${OLD_BLOCK}; on-disk db.version=${OLD_DBVER}"

# ---- Phase 2: open the same database with the current binary ----------------

log "Phase 2: current binary opens the same --db-dir with --db.auto-migrate"
"$CURRENT_KATANA" --db-dir "$DB" --db.auto-migrate --http.addr 127.0.0.1 --http.port "$PORT" \
    > "${WORKDIR}/katana-current.log" 2>&1 &
NEW_PID=$!
wait_rpc "$PORT" "$NEW_PID" || { tail -n 60 "${WORKDIR}/katana-current.log" >&2; fail "current katana failed to open the baseline database"; }

NEW_CHAINID="$(rpc_result "$PORT" starknet_chainId)" || fail "current: starknet_chainId failed"
NEW_BLOCK="$(rpc_result "$PORT" starknet_blockNumber)" || fail "current: starknet_blockNumber failed"
rpc_result "$PORT" starknet_getBlockWithTxHashes "[{\"block_number\": ${OLD_BLOCK}}]" >/dev/null \
    || fail "current: block ${OLD_BLOCK} written by the baseline is missing after open"

stop_katana "$NEW_PID"; NEW_PID=""
NEW_DBVER="$(read_db_version "$DB")"

# ---- Assertions -------------------------------------------------------------

[[ "$NEW_CHAINID" == "$OLD_CHAINID" ]] \
    || fail "chain id changed (${OLD_CHAINID} -> ${NEW_CHAINID}): current binary did not load the baseline chain"
[[ "$NEW_BLOCK" -ge "$OLD_BLOCK" ]] \
    || fail "block height regressed (${OLD_BLOCK} -> ${NEW_BLOCK}): state did not survive the upgrade (reset to fresh genesis?)"

log "PASS: current katana opened the database written by ${PREV_TAG:-baseline binary}"
log "      chain ${OLD_CHAINID}, block ${OLD_BLOCK} preserved (now ${NEW_BLOCK}); db.version ${OLD_DBVER} -> ${NEW_DBVER}"
