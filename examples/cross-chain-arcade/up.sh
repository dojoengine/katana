#!/usr/bin/env bash
# Bring up the cross-chain arcade demo (L1 -> L2 fan-out):
#
#   settlement Katana ("L1", :5050, plain --dev)
#     + piltover core contract   (deployed by `katana init rollup`)
#     + arcade dispenser contract
#   appchain Katana ("L2", :5051, booted from the generated rollup config,
#     --messaging.enabled) + N machine contracts (each an insert_coin l1_handler)
#   verify.ts (the PR #623 gate) + React frontend (:3001)
#
#   arcade.play_all -> one L1 tx -> N messages to N DISTINCT machine contracts
#   -> relayed as N L1HandlerTx on the appchain (all of them, thanks to #623).
#
# We use validity-proof mode with a dummy fact registry: this demo never settles
# (no L2 -> L1, no update_state), so the fact registry is irrelevant — it just
# lets `init rollup` deploy a piltover core the appchain's startup validation
# accepts. No TEE registry, no embedded settlement, no Torii, no Dojo.
#
# Ctrl-C tears down both nodes.
set -euo pipefail

DEMO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$DEMO_DIR/../.." && pwd)"
RUN_DIR="$DEMO_DIR/.run"
CHAIN_DIR="$RUN_DIR/chain-config"
mkdir -p "$RUN_DIR" "$DEMO_DIR/app/src"

# Katana's first deterministic --dev account (account 0). Used on the settlement
# node as the init-rollup operator + the arcade deployer / play_all signer.
DEV0_ADDR="0x127fd5f1fe78a71f8bcd1fec63e3fe2f0486b6ecd5c86a0466c3a21fa5cfcec"
DEV0_PK="0xc5b2fcab997346f3ea1c00b002ecf6f382c5f9c9659a3894eb783c5320f912"

fail() { echo "error: $1" >&2; exit 1; }

# ── Preflight ───────────────────────────────────────────────────────────────
echo "→ preflight…"
# katana — always the binary built from THIS repo (the fix under test lives here),
# never a released PATH/asdf katana.
KATANA="$REPO_ROOT/target/debug/katana"
if [[ ! -x "$KATANA" ]]; then
  echo "  katana not found at target/debug — building (cargo build -p katana)…"
  ( cd "$REPO_ROOT" && cargo build -p katana --bin katana ) \
    || fail "failed to build katana. Build it manually:  ( cd \"$REPO_ROOT\" && cargo build -p katana --bin katana )"
fi
[[ -x "$KATANA" ]] || fail "katana still not found at $KATANA after build."
command -v bun >/dev/null 2>&1 || fail "'bun' not found on PATH. Install it: https://bun.sh"
command -v node >/dev/null 2>&1 || fail "'node' not found on PATH (used to read the generated genesis)."
command -v scarb >/dev/null 2>&1 || fail "'scarb' not found on PATH. Install it (see .tool-versions):  asdf install"
echo "→ katana: $KATANA"
echo "→ scarb:  $(scarb --version 2>&1 | head -1)"

echo "→ installing JS dependencies…"
( cd "$DEMO_DIR" && bun install >/dev/null )
( cd "$DEMO_DIR/app" && bun install >/dev/null )

SETTLEMENT_PID=""; APPCHAIN_PID=""
cleanup() {
  echo ""; echo "→ shutting down…"
  [[ -n "$APPCHAIN_PID" ]] && kill "$APPCHAIN_PID" 2>/dev/null || true
  [[ -n "$SETTLEMENT_PID" ]] && kill "$SETTLEMENT_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

# ── 1. Settlement node ("L1") ───────────────────────────────────────────────
echo "→ starting settlement node on :5050…"
"$KATANA" --dev --dev.no-fee --http.port 5050 --http.cors_origins '*' --explorer \
  > "$RUN_DIR/settlement.log" 2>&1 &
SETTLEMENT_PID=$!
until curl -s -o /dev/null http://localhost:5050/ 2>/dev/null; do sleep 0.5; done

# ── 2. Deploy the piltover core + generate the rollup config (before the
#       appchain — it boots from this config) ───────────────────────────────
echo "→ deploying piltover core (katana init rollup, validity mode)…"
rm -rf "$CHAIN_DIR"
"$KATANA" init rollup \
  --id ARCADE \
  --settlement-chain http://localhost:5050 \
  --settlement-account-address "$DEV0_ADDR" \
  --settlement-account-private-key "$DEV0_PK" \
  --settlement-facts-registry 0x1 \
  --output-path "$CHAIN_DIR" > "$RUN_DIR/init.log" 2>&1
PILTOVER=$(sed -nE 's/^core_contract = "(0x[0-9a-fA-F]+)".*/\1/p' "$CHAIN_DIR/config.toml")
[[ -n "$PILTOVER" ]] || { echo "error: could not parse piltover from config.toml" >&2; cat "$RUN_DIR/init.log" >&2; exit 1; }
echo "   piltover=$PILTOVER"

# ── 3. Write base deployments.json (settlement = dev acct 0; appchain = the
#       generated rollup genesis account) ─────────────────────────────────────
echo "→ writing base deployments.json…"
node -e '
  const fs = require("node:fs");
  const g = require(process.argv[1]);
  const [addr, acct] = Object.entries(g.accounts)[0];
  const d = {
    settlement: {
      rpcUrl: "http://localhost:5050", explorer: "http://localhost:5050/explorer",
      account: { address: process.argv[3], privateKey: process.argv[4] },
      piltover: process.argv[5],
    },
    appchain: {
      rpcUrl: "http://localhost:5051", explorer: "http://localhost:5051/explorer",
      account: { address: addr, privateKey: acct.privateKey },
    },
  };
  fs.writeFileSync(process.argv[2], JSON.stringify(d, null, 2) + "\n");
' "$CHAIN_DIR/genesis.json" "$DEMO_DIR/app/src/deployments.json" "$DEV0_ADDR" "$DEV0_PK" "$PILTOVER"

# ── 4. Appchain node ("L2"), booted from the rollup config, relaying L1 -> L2 ─
# --block-time 3000: interval mining every 3s, so relayed L1Handler txs land on a
# steady cadence. The settlement layer + core contract come from --chain, so no
# --settlement.* flags here; --messaging.enabled turns on the L1 -> L2 relay.
echo "→ starting appchain node on :5051…"
"$KATANA" --chain "$CHAIN_DIR" --dev --dev.no-fee --block-time 3000 \
  --http.port 5051 --http.cors_origins '*' --explorer --messaging.enabled \
  > "$RUN_DIR/appchain.log" 2>&1 &
APPCHAIN_PID=$!
until curl -s -o /dev/null http://localhost:5051/ 2>/dev/null; do sleep 0.5; done

# ── 5. Deploy the game contracts (machines on L2, arcade on L1) ──────────────
( cd "$DEMO_DIR" && bun run scripts/deploy-game.ts )

# ── 6. The PR #623 gate: one play_all must reach EVERY machine ───────────────
echo "→ verifying the fan-out (PR #623 gate)…"
if ( cd "$DEMO_DIR" && bun run scripts/verify.ts ); then
  echo "   ✅ verification passed."
else
  echo "   ⚠️  verification FAILED — some machines never received their coin." >&2
  echo "      This is the pre-#623 behavior: check that the katana under test" >&2
  echo "      includes the L1-handler nonce-gate fix. Serving the UI anyway…" >&2
fi

echo ""
echo "✓ Demo is up:"
echo "    settlement RPC : http://localhost:5050   explorer: http://localhost:5050/explorer"
echo "    appchain RPC   : http://localhost:5051   explorer: http://localhost:5051/explorer"
echo "    frontend       : http://localhost:3001"
echo ""

# ── 7. Frontend (foreground; Ctrl-C stops everything) ───────────────────────
( cd "$DEMO_DIR/app" && exec bun run dev )
