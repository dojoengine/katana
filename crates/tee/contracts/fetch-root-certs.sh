#!/usr/bin/env bash
#
# Fetch AMD root certificate hashes from KDS via the `kds-client` binary
# and emit them in the flat low/high schema expected by snforge's
# `FileParser::<RootCerts>::parse_json` (see amd_tee_registry/tests/root_certs_helper.cairo).
#
# Default output: crates/tee/contracts/amd_root_certs.json
#
# Usage:
#   crates/tee/contracts/fetch-root-certs.sh [output-path]

set -euo pipefail

if ! command -v jq >/dev/null 2>&1; then
    echo "error: jq is required" >&2
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
OUT="${1:-$SCRIPT_DIR/amd_root_certs.json}"

RAW="$(mktemp)"
trap 'rm -f "$RAW"' EXIT

(
    cd "$REPO_ROOT"
    cargo run --quiet -p katana-tee --bin kds-client -- \
        fetch --processors milan,genoa --output "$RAW"
)

mkdir -p "$(dirname "$OUT")"

# Each ark_hash is "0x" + 64 hex chars (32 bytes). Split into 128-bit halves:
# high = top 16 bytes, low = bottom 16 bytes.
jq '
  def split(h):
    (h | sub("^0x"; "")) as $x
    | { high: ("0x" + $x[0:32]), low: ("0x" + $x[32:64]) };
  split(.milan.ark_hash) as $m
  | split(.genoa.ark_hash) as $g
  | {
      genoa_ark_hash_high: $g.high,
      genoa_ark_hash_low:  $g.low,
      milan_ark_hash_high: $m.high,
      milan_ark_hash_low:  $m.low,
  }
' "$RAW" > "$OUT"

echo "wrote $OUT"
