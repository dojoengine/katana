#!/usr/bin/env bash
set -euo pipefail

# Declare + deploy the mock AMD TEE registry to a Katana node.
#
# The mock registry (`piltover_mock_amd_tee_registry`) accepts any SP1 proof
# without verification. It's the permissive attestation verifier used to
# exercise the TEE settlement path locally / in e2e tests — do NOT deploy it to
# production.
#
# This is a self-contained replacement for the
# `saya-ops core-contract declare-and-deploy-tee-registry-mock` bootstrap
# helper: it declares + deploys the artifact built from THIS repo directly via
# `starkli`, with no saya dependency.
#
# Configuration is via environment variables (all have sensible defaults that
# target a fresh `katana --dev` node and its first predeployed dev account):
#
#   RPC_URL          Katana JSON-RPC endpoint   (default: http://localhost:5050)
#   ACCOUNT_ADDRESS  Funded account to pay fees (default: katana dev account #0)
#   PRIVATE_KEY      Private key for that account
#   SALT             UDC deploy salt            (default: 0x7ee)
#
# On success the deployed registry address is printed to stdout as:
#   TEE registry mock address: 0x...
# so callers can parse it the same way they parsed saya-ops' output.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

BUILD_DIR="$REPO_ROOT/crates/contracts/build"
SIERRA_FILE="$BUILD_DIR/piltover_mock_amd_tee_registry.contract_class.json"
CASM_FILE="$BUILD_DIR/piltover_mock_amd_tee_registry.compiled_contract_class.json"

RPC_URL="${RPC_URL:-http://localhost:5050}"
# katana's first predeployed dev account (deterministic across `--dev` runs).
ACCOUNT_ADDRESS="${ACCOUNT_ADDRESS:-0x127fd5f1fe78a71f8bcd1fec63e3fe2f0486b6ecd5c86a0466c3a21fa5cfcec}"
PRIVATE_KEY="${PRIVATE_KEY:-0xc5b2fcab997346f3ea1c00b002ecf6f382c5f9c9659a3894eb783c5320f912}"
SALT="${SALT:-0x7ee}"

fail() { echo "error: $*" >&2; exit 1; }

# `starkli` is the standard tool for declaring a raw contract class artifact.
command -v starkli >/dev/null 2>&1 \
  || fail "'starkli' not found on PATH. Install it: https://github.com/xJonathanLEI/starkli"

# Gracefully handle the contract not being built yet. The mock registry is built
# alongside the rest of the piltover contracts by the 'contracts' Makefile target.
if [[ ! -f "$SIERRA_FILE" || ! -f "$CASM_FILE" ]]; then
  echo "error: mock TEE registry artifact not found at" >&2
  echo "         $SIERRA_FILE" >&2
  echo "" >&2
  echo "The contract hasn't been built yet. Build it from the repo root with:" >&2
  echo "" >&2
  echo "         make contracts" >&2
  echo "" >&2
  exit 1
fi

# Class hash is computed offline from the Sierra artifact — deterministic, and
# needed for the deploy step regardless of whether the declare is a no-op.
CLASS_HASH="$(starkli class-hash "$SIERRA_FILE")"
echo "→ mock TEE registry class hash: $CLASS_HASH"

# starkli needs an account descriptor; fetch one for the funded account from the
# node (the account is predeployed on a `katana --dev` chain).
ACCOUNT_FILE="$(mktemp -t tee-registry-account.XXXXXX.json)"
trap 'rm -f "$ACCOUNT_FILE"' EXIT
starkli account fetch "$ACCOUNT_ADDRESS" --rpc "$RPC_URL" --output "$ACCOUNT_FILE" >/dev/null \
  || fail "could not fetch account $ACCOUNT_ADDRESS from $RPC_URL (is the node up?)."

# Declare (idempotent: starkli is a no-op if the class is already declared).
# Pass the prebuilt CASM so the compiled class hash matches scarb's output rather
# than relying on starkli's bundled compiler.
echo "→ declaring mock TEE registry…"
starkli declare \
  --account "$ACCOUNT_FILE" \
  --private-key "$PRIVATE_KEY" \
  --rpc "$RPC_URL" \
  --casm-file "$CASM_FILE" \
  "$SIERRA_FILE" >/dev/null

# Deploy. No constructor args — the mock takes none.
echo "→ deploying mock TEE registry (salt=$SALT)…"
DEPLOY_OUT="$(starkli deploy \
  --account "$ACCOUNT_FILE" \
  --private-key "$PRIVATE_KEY" \
  --rpc "$RPC_URL" \
  --salt "$SALT" \
  "$CLASS_HASH" 2>&1)"

# starkli prints the deployed address on its own line (0x-prefixed felt).
ADDRESS="$(echo "$DEPLOY_OUT" | grep -oE '0x[0-9a-fA-F]{1,64}' | tail -1)"
[[ -n "$ADDRESS" ]] || { echo "$DEPLOY_OUT" >&2; fail "could not parse deployed address from starkli output."; }

echo "TEE registry mock address: $ADDRESS"
