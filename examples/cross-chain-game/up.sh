#!/usr/bin/env bash
# Bring up the full cross-chain game store demo (both directions):
#
#   settlement Katana ("L1", SN_SEPOLIA)
#     + piltover core contract        (deployed by `katana init rollup --tee`)
#     + mock TEE registry             (deployed by `saya-ops`)
#   appchain Katana ("L2", rollup, --tee mock) settling to piltover
#   saya-tee --mock-prove sidecar     (proves appchain blocks, drives settlement)
#   demo contracts (game_minter, achievements, score_registry)
#   React frontend
#
#   L1 -> L2: piltover.send_message_to_appchain -> appchain mint_game
#   L2 -> L1: appchain send_message_to_l1 -> saya settles -> score_registry consumes
#
# Ctrl-C tears down the settlement/appchain nodes and the saya-tee sidecar.
set -euo pipefail

DEMO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$DEMO_DIR/../.." && pwd)"
RUN_DIR="$DEMO_DIR/.run"
CHAIN_DIR="$RUN_DIR/chain-config"
mkdir -p "$RUN_DIR"

# Deterministic seed-0 dev account on the settlement node (used as the saya /
# bootstrap / purchase account).
SETTLE_ADDR="0x127fd5f1fe78a71f8bcd1fec63e3fe2f0486b6ecd5c86a0466c3a21fa5cfcec"
SETTLE_PK="0xc5b2fcab997346f3ea1c00b002ecf6f382c5f9c9659a3894eb783c5320f912"
TEE_REGISTRY_SALT="0x7ee"

if [[ -x "$REPO_ROOT/target/release/katana" ]]; then KATANA="$REPO_ROOT/target/release/katana"
elif [[ -x "$REPO_ROOT/target/debug/katana" ]]; then KATANA="$REPO_ROOT/target/debug/katana"
elif command -v katana >/dev/null 2>&1; then KATANA="$(command -v katana)"
else echo "error: katana binary not found. Run 'cargo build --release' first." >&2; exit 1; fi

for bin in saya-ops saya-tee; do
  command -v "$bin" >/dev/null 2>&1 || {
    echo "error: '$bin' not found on PATH. Install from cartridge-gg/saya (v0.4.0)." >&2; exit 1; }
done
echo "→ katana: $KATANA"
echo "→ saya-tee: $(command -v saya-tee)"

SETTLEMENT_PID=""; APPCHAIN_PID=""; SAYA_PID=""
cleanup() {
  echo ""; echo "→ shutting down…"
  [[ -n "$SAYA_PID" ]] && kill "$SAYA_PID" 2>/dev/null || true
  [[ -n "$APPCHAIN_PID" ]] && kill "$APPCHAIN_PID" 2>/dev/null || true
  [[ -n "$SETTLEMENT_PID" ]] && kill "$SETTLEMENT_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

echo "→ installing JS dependencies…"
( cd "$DEMO_DIR" && bun install >/dev/null )
( cd "$DEMO_DIR/app" && bun install >/dev/null )

echo "→ building contracts (scarb)…"
( cd "$DEMO_DIR/cairo" && scarb build )

# 1. Settlement node (SN_SEPOLIA so saya-ops / init rollup chain id match).
echo "→ starting settlement node on :5050…"
"$KATANA" --dev --dev.no-fee --chain-id SN_SEPOLIA --http.port 5050 \
  --http.cors_origins '*' --explorer > "$RUN_DIR/settlement.log" 2>&1 &
SETTLEMENT_PID=$!
until curl -s -o /dev/null http://localhost:5050/ 2>/dev/null; do sleep 0.5; done

# 2. Mock TEE registry on settlement (permissive attestation verifier).
echo "→ deploying mock TEE registry (saya-ops)…"
REG_OUT=$(SETTLEMENT_RPC_URL=http://localhost:5050 \
  SETTLEMENT_ACCOUNT_ADDRESS="$SETTLE_ADDR" \
  SETTLEMENT_ACCOUNT_PRIVATE_KEY="$SETTLE_PK" \
  SETTLEMENT_CHAIN_ID=SN_SEPOLIA \
  saya-ops core-contract declare-and-deploy-tee-registry-mock --salt "$TEE_REGISTRY_SALT" 2>&1)
TEE_REGISTRY=$(echo "$REG_OUT" | sed -nE 's/.*TEE registry mock address:[[:space:]]*(0x[0-9a-fA-F]+).*/\1/p' | tail -1)
[[ -n "$TEE_REGISTRY" ]] || { echo "error: could not parse TEE registry address:" >&2; echo "$REG_OUT" >&2; exit 1; }
echo "   tee_registry=$TEE_REGISTRY"

# 3. Deploy + configure the piltover core on settlement and write the rollup chain config.
echo "→ deploying piltover core + generating rollup config (katana init rollup --tee)…"
rm -rf "$CHAIN_DIR"
"$KATANA" init rollup \
  --id GAMECHAIN \
  --settlement-chain http://localhost:5050 \
  --settlement-account-address "$SETTLE_ADDR" \
  --settlement-account-private-key "$SETTLE_PK" \
  --tee \
  --tee-registry-address "$TEE_REGISTRY" \
  --output-path "$CHAIN_DIR" > "$RUN_DIR/init.log" 2>&1
PILTOVER=$(sed -nE 's/^core_contract = "(0x[0-9a-fA-F]+)".*/\1/p' "$CHAIN_DIR/config.toml")
[[ -n "$PILTOVER" ]] || { echo "error: could not parse piltover address from config.toml" >&2; cat "$RUN_DIR/init.log" >&2; exit 1; }
echo "   piltover=$PILTOVER"

# 4. Write the base deployments.json (rpc urls, accounts, piltover). The appchain
#    account comes from the generated rollup genesis.
echo "→ writing base deployments.json…"
node -e '
  const fs = require("node:fs");
  const g = require(process.argv[1]);
  const [addr, acct] = Object.entries(g.accounts)[0];
  const d = {
    settlement: {
      rpcUrl: "http://localhost:5050", explorer: "http://localhost:5050/explorer",
      account: { address: process.argv[2], privateKey: process.argv[3] },
      piltover: process.argv[4],
    },
    appchain: {
      rpcUrl: "http://localhost:5051", explorer: "http://localhost:5051/explorer",
      account: { address: addr, privateKey: acct.privateKey },
    },
  };
  fs.writeFileSync(process.argv[5], JSON.stringify(d, null, 2) + "\n");
' "$CHAIN_DIR/genesis.json" "$SETTLE_ADDR" "$SETTLE_PK" "$PILTOVER" "$DEMO_DIR/app/src/deployments.json"

# 5. Appchain rollup node, settling to piltover, with L1->L2 messaging enabled.
echo "→ starting appchain node on :5051…"
# --dev --dev.no-fee disables fees on the rollup (mirrors the saya-tee test
# harness' fee:false config); without it, fee estimation on the near-empty
# rollup produces resource bounds below the actual gas price and txs revert.
"$KATANA" --chain "$CHAIN_DIR" --tee mock --dev --dev.no-fee --http.port 5051 \
  --http.cors_origins '*' --explorer --messaging.enabled \
  > "$RUN_DIR/appchain.log" 2>&1 &
APPCHAIN_PID=$!
until curl -s -o /dev/null http://localhost:5051/ 2>/dev/null; do sleep 0.5; done

# 6. saya-tee sidecar: proves appchain blocks and submits state updates (which
#    carry L2->L1 message hashes) to the piltover core.
echo "→ starting saya-tee --mock-prove sidecar…"
rm -rf "$RUN_DIR/saya-db"
saya-tee tee start --mock-prove \
  --rollup-rpc http://localhost:5051 \
  --settlement-rpc http://localhost:5050 \
  --settlement-piltover-address "$PILTOVER" \
  --tee-registry-address "$TEE_REGISTRY" \
  --settlement-account-address "$SETTLE_ADDR" \
  --settlement-account-private-key "$SETTLE_PK" \
  --prover-private-key 0xdeadbeef \
  --db-dir "$RUN_DIR/saya-db" \
  --batch-size 1 \
  --attestor-poll-interval-ms 1000 \
  > "$RUN_DIR/saya.log" 2>&1 &
SAYA_PID=$!

# 7. Deploy the demo contracts and fill in their addresses.
( cd "$DEMO_DIR" && bun run scripts/deploy.ts )

echo ""
echo "✓ Demo is up:"
echo "    settlement RPC : http://localhost:5050   explorer: http://localhost:5050/explorer"
echo "    appchain RPC   : http://localhost:5051   explorer: http://localhost:5051/explorer"
echo "    saya-tee       : running (.run/saya.log)"
echo "    frontend       : http://localhost:3001"
echo ""

# 8. Frontend (foreground; Ctrl-C stops everything).
( cd "$DEMO_DIR/app" && exec bun run dev )
