#!/usr/bin/env bash
# Bring up the full cross-chain game store demo:
#   1. build the appchain Cairo contract
#   2. start the settlement ("L1") Katana node
#   3. deploy the messaging contract on it
#   4. start the appchain ("L2") Katana node wired to that messaging contract
#   5. deploy the game_minter contract on the appchain
#   6. start the React frontend (foreground)
#
# Ctrl-C tears down the Katana nodes started by this script.
set -euo pipefail

DEMO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$DEMO_DIR/../.." && pwd)"
RUN_DIR="$DEMO_DIR/.run"
mkdir -p "$RUN_DIR"

# Locate the katana binary.
if [[ -x "$REPO_ROOT/target/release/katana" ]]; then
  KATANA="$REPO_ROOT/target/release/katana"
elif [[ -x "$REPO_ROOT/target/debug/katana" ]]; then
  KATANA="$REPO_ROOT/target/debug/katana"
elif command -v katana >/dev/null 2>&1; then
  KATANA="$(command -v katana)"
else
  echo "error: katana binary not found. Run 'cargo build --release' first." >&2
  exit 1
fi
echo "→ using katana: $KATANA"

SETTLEMENT_PID=""
APPCHAIN_PID=""
cleanup() {
  echo ""
  echo "→ shutting down Katana nodes…"
  [[ -n "$APPCHAIN_PID" ]] && kill "$APPCHAIN_PID" 2>/dev/null || true
  [[ -n "$SETTLEMENT_PID" ]] && kill "$SETTLEMENT_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

# 0. dependencies
echo "→ installing JS dependencies…"
( cd "$DEMO_DIR" && bun install >/dev/null )
( cd "$DEMO_DIR/app" && bun install >/dev/null )

# 1. build the appchain contract
echo "→ building appchain contract (scarb)…"
( cd "$DEMO_DIR/cairo" && scarb build )

# 2. start settlement node
echo "→ starting settlement node on :5050…"
"$KATANA" --dev --dev.no-fee --http.port 5050 --http.cors_origins '*' --explorer > "$RUN_DIR/settlement.log" 2>&1 &
SETTLEMENT_PID=$!

# 3. deploy messaging contract on settlement
( cd "$DEMO_DIR" && bun run scripts/deploy-settlement.ts )

MOCK=$(node -e "console.log(require('$DEMO_DIR/app/src/deployments.json').settlement.messagingContract)")
if [[ -z "$MOCK" ]]; then echo "error: messaging contract not deployed" >&2; exit 1; fi

# 4. start appchain node wired to the messaging contract
echo "→ starting appchain node on :5051 (settlement core-contract: $MOCK)…"
"$KATANA" --dev --dev.no-fee --http.port 5051 --http.cors_origins '*' --explorer \
  --messaging.enabled \
  --settlement.chain starknet \
  --settlement.rpc-url http://localhost:5050 \
  --settlement.core-contract "$MOCK" \
  --messaging.from-block 0 \
  > "$RUN_DIR/appchain.log" 2>&1 &
APPCHAIN_PID=$!

# 5. deploy game_minter on appchain
( cd "$DEMO_DIR" && bun run scripts/deploy-appchain.ts )

echo ""
echo "✓ Demo is up:"
echo "    settlement RPC : http://localhost:5050   (logs: .run/settlement.log)"
echo "    appchain RPC   : http://localhost:5051   (logs: .run/appchain.log)"
echo "    frontend       : http://localhost:3001"
echo ""

# 6. frontend (foreground; Ctrl-C stops everything)
( cd "$DEMO_DIR/app" && exec bun run dev )
