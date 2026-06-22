#!/usr/bin/env bash
set -euo pipefail

# Declare + deploy the mock AMD TEE registry to a Katana node.
#
# The mock registry (`mock_amd_tee_registry`) accepts any SP1 proof without
# verification. It's the permissive attestation verifier used to exercise the
# TEE settlement path locally / in e2e tests — do NOT deploy it to production.
#
# The mock registry is compiled into the `katana` binary as an embedded class,
# so this script drives `katana bootstrap` directly: no prebuilt contract
# artifacts and no external tooling (`starkli`) are required. Both `katana
# bootstrap declare` and `katana bootstrap deploy` are idempotent — re-running
# against the same chain converges without error.
#
# Configuration is via environment variables (all have sensible defaults that
# target a fresh `katana --dev` node and its first predeployed dev account):
#
#   RPC_URL          Katana JSON-RPC endpoint   (default: http://localhost:5050)
#   ACCOUNT_ADDRESS  Funded account to pay fees (default: katana dev account #0)
#   PRIVATE_KEY      Private key for that account
#   SALT             UDC deploy salt            (default: 0x7ee)
#   KATANA_BIN       katana binary to invoke    (default: repo build, else PATH)
#
# On success the deployed registry address is printed to stdout as:
#   TEE registry mock address: 0x...
# so callers can parse it the same way they parsed saya-ops' output.

# Embedded class name registered in `crates/bootstrap/src/embedded.rs`.
CLASS_NAME="mock_amd_tee_registry"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

RPC_URL="${RPC_URL:-http://localhost:5050}"
# katana's first predeployed dev account (deterministic across `--dev` runs).
ACCOUNT_ADDRESS="${ACCOUNT_ADDRESS:-0x127fd5f1fe78a71f8bcd1fec63e3fe2f0486b6ecd5c86a0466c3a21fa5cfcec}"
PRIVATE_KEY="${PRIVATE_KEY:-0xc5b2fcab997346f3ea1c00b002ecf6f382c5f9c9659a3894eb783c5320f912}"
SALT="${SALT:-0x7ee}"

fail() { echo "error: $*" >&2; exit 1; }

# Resolve the katana binary. Prefer a binary built from THIS repo (it carries the
# embedded `mock_amd_tee_registry` class), then fall back to one on PATH.
KATANA_BIN="${KATANA_BIN:-}"
if [[ -z "$KATANA_BIN" ]]; then
  for cand in "$REPO_ROOT/target/release/katana" "$REPO_ROOT/target/debug/katana"; do
    if [[ -x "$cand" ]]; then KATANA_BIN="$cand"; break; fi
  done
fi
KATANA_BIN="${KATANA_BIN:-katana}"
command -v "$KATANA_BIN" >/dev/null 2>&1 \
  || fail "katana binary '$KATANA_BIN' not found. Build it ('cargo build --release') or set KATANA_BIN."

# Declare (idempotent: bootstrap skips the declare if the class is already on-chain).
echo "→ declaring mock TEE registry ($CLASS_NAME)…"
"$KATANA_BIN" bootstrap \
  --rpc-url "$RPC_URL" \
  --account "$ACCOUNT_ADDRESS" \
  --private-key "$PRIVATE_KEY" \
  declare "$CLASS_NAME"

# Deploy (idempotent: bootstrap skips if a contract is already at the deterministic
# address). No constructor args — the mock takes none. `--json` gives a stable,
# parseable report instead of the human-readable tables.
echo "→ deploying mock TEE registry (salt=$SALT)…"
DEPLOY_JSON="$("$KATANA_BIN" bootstrap --json \
  --rpc-url "$RPC_URL" \
  --account "$ACCOUNT_ADDRESS" \
  --private-key "$PRIVATE_KEY" \
  deploy "$CLASS_NAME:salt=$SALT")"

# Pull the deployed address out of the JSON report. Exactly one contract is
# deployed, so the first "address" field is ours. (Avoids a hard `jq` dependency.)
ADDRESS="$(printf '%s' "$DEPLOY_JSON" \
  | grep -oE '"address":"0x[0-9a-fA-F]+"' \
  | head -1 \
  | grep -oE '0x[0-9a-fA-F]+')"
[[ -n "$ADDRESS" ]] || { printf '%s\n' "$DEPLOY_JSON" >&2; fail "could not parse deployed address from bootstrap output."; }

echo "TEE registry mock address: $ADDRESS"
