#!/usr/bin/env bash
# Bring up the cross-chain-dungeon demo. Unlike cross-chain-game, the settlement
# layer is REAL Starknet Sepolia (remote) — only the appchain runs locally:
#
#   Starknet Sepolia (remote)
#     + piltover core         (deployed by `katana init rollup --tee`)
#     + mock TEE registry     (deployed by `saya-ops`)
#     + GAME_TOKEN / TokenSale / Entry / score world  (deployed by scripts/deploy.ts)
#   appchain Katana (:5070, rollup, --tee mock) settling to piltover on Sepolia
#   saya-tee --mock-prove sidecar (proves appchain blocks → update_state on Sepolia)
#   two torii indexers (Sepolia score :8091, appchain game :8092)
#   React frontend (:3002)
#
# Requires a funded Sepolia operator + saya account and a USDC address — see
# .env.example (copy to .env). These deploys cost real Sepolia gas.
#
# Ctrl-C tears down the appchain node, the saya-tee sidecar, and the toriis.
set -euo pipefail

DEMO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$DEMO_DIR/../.." && pwd)"
RUN_DIR="$DEMO_DIR/.run"
CHAIN_DIR="$RUN_DIR/chain-config"
mkdir -p "$RUN_DIR"

# Ports — distinct from cross-chain-game (5051/8081/8082/3001) so both can run.
APPCHAIN_PORT=5070
TORII_SCORE_HTTP=8091; TORII_SCORE_GRPC=50091; TORII_SCORE_RELAY=9191
TORII_GAME_HTTP=8092;  TORII_GAME_GRPC=50092;  TORII_GAME_RELAY=9194
FRONTEND_PORT=3002
TEE_REGISTRY_SALT="0x7ee"

fail() { echo "error: $1" >&2; exit 1; }
DOJO_DIR="$REPO_ROOT/../dojo"

# ── Load .env ──────────────────────────────────────────────────────────────────
[[ -f "$DEMO_DIR/.env" ]] || fail "no .env — copy .env.example to .env and fill in the Sepolia operator/saya accounts + USDC address."
set -a; # shellcheck disable=SC1091
source "$DEMO_DIR/.env"; set +a
for v in SEPOLIA_RPC_URL OPERATOR_ADDRESS OPERATOR_PRIVATE_KEY SAYA_ADDRESS SAYA_PRIVATE_KEY USDC_ADDRESS; do
  [[ -n "${!v:-}" ]] || fail "missing $v in .env (see .env.example)."
done

# ── Preflight ────────────────────────────────────────────────────────────────
echo "→ preflight…"
if command -v asdf >/dev/null 2>&1; then
  ( cd "$DEMO_DIR" && asdf install ) || echo "  warning: 'asdf install' had issues; verifying tools below…" >&2
else
  echo "  warning: asdf not found — install it, or put sozo/torii/scarb on PATH (see .tool-versions)." >&2
fi

if [[ -x "$REPO_ROOT/target/release/katana" ]]; then KATANA="$REPO_ROOT/target/release/katana"
elif [[ -x "$REPO_ROOT/target/debug/katana" ]]; then KATANA="$REPO_ROOT/target/debug/katana"
elif command -v katana >/dev/null 2>&1; then KATANA="$(command -v katana)"
else fail "katana binary not found. Build it:  ( cd \"$REPO_ROOT\" && cargo build --release )"; fi

for bin in saya-ops saya-tee; do
  command -v "$bin" >/dev/null 2>&1 || fail "'$bin' not found on PATH. Install the patched saya v0.4.0 — see ../cross-chain-game/saya-patch/README.md."
done
for bin in sozo torii scarb; do
  command -v "$bin" >/dev/null 2>&1 || fail "'$bin' not found on PATH. Run 'asdf install' in this directory (see .tool-versions)."
done
[[ -d "$DOJO_DIR/crates/dojo/core" ]] || fail "dojo checkout not found at $DOJO_DIR — the cairo packages depend on it by path (clone it as a sibling, ref sozo/v1.8.7)."
echo "→ katana: $KATANA"
echo "→ settlement: Starknet Sepolia ($SEPOLIA_RPC_URL)"

APPCHAIN_PID=""; SAYA_PID=""; TORII_SCORE_PID=""; TORII_GAME_PID=""
cleanup() {
  echo ""; echo "→ shutting down…"
  [[ -n "$TORII_GAME_PID" ]] && kill "$TORII_GAME_PID" 2>/dev/null || true
  [[ -n "$TORII_SCORE_PID" ]] && kill "$TORII_SCORE_PID" 2>/dev/null || true
  [[ -n "$SAYA_PID" ]] && kill "$SAYA_PID" 2>/dev/null || true
  [[ -n "$APPCHAIN_PID" ]] && kill "$APPCHAIN_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

echo "→ installing JS dependencies…"
( cd "$DEMO_DIR" && bun install >/dev/null )
( cd "$DEMO_DIR/app" && bun install >/dev/null )

# 1+2. Mock TEE registry + piltover core on Sepolia — both gas-costing real
#      deploys. Skip them if a previous run already bootstrapped this chain dir
#      (set FRESH=1 to force a fresh bootstrap). saya is the piltover operator
#      (the only update_state caller) and must differ from the operator account.
if [[ -z "${FRESH:-}" && -f "$CHAIN_DIR/config.toml" && -f "$RUN_DIR/tee_registry" ]]; then
  echo "→ reusing existing Sepolia bootstrap (set FRESH=1 to redeploy)…"
  TEE_REGISTRY=$(cat "$RUN_DIR/tee_registry")
else
  echo "→ deploying mock TEE registry on Sepolia (saya-ops)…"
  REG_OUT=$(SETTLEMENT_RPC_URL="$SEPOLIA_RPC_URL" \
    SETTLEMENT_ACCOUNT_ADDRESS="$OPERATOR_ADDRESS" \
    SETTLEMENT_ACCOUNT_PRIVATE_KEY="$OPERATOR_PRIVATE_KEY" \
    SETTLEMENT_CHAIN_ID=SN_SEPOLIA \
    saya-ops core-contract declare-and-deploy-tee-registry-mock --salt "$TEE_REGISTRY_SALT" 2>&1)
  TEE_REGISTRY=$(echo "$REG_OUT" | sed -nE 's/.*TEE registry mock address:[[:space:]]*(0x[0-9a-fA-F]+).*/\1/p' | tail -1)
  [[ -n "$TEE_REGISTRY" ]] || { echo "error: could not parse TEE registry address:" >&2; echo "$REG_OUT" >&2; exit 1; }
  echo "$TEE_REGISTRY" > "$RUN_DIR/tee_registry"

  echo "→ deploying piltover core + generating rollup config (katana init rollup --tee)…"
  rm -rf "$CHAIN_DIR"
  "$KATANA" init rollup \
    --id DUNGEON \
    --settlement-chain "$SEPOLIA_RPC_URL" \
    --settlement-account-address "$SAYA_ADDRESS" \
    --settlement-account-private-key "$SAYA_PRIVATE_KEY" \
    --tee \
    --tee-registry-address "$TEE_REGISTRY" \
    --output-path "$CHAIN_DIR" > "$RUN_DIR/init.log" 2>&1
fi
echo "   tee_registry=$TEE_REGISTRY"
PILTOVER=$(sed -nE 's/^core_contract = "(0x[0-9a-fA-F]+)".*/\1/p' "$CHAIN_DIR/config.toml")
[[ -n "$PILTOVER" ]] || { echo "error: could not parse piltover address from config.toml" >&2; cat "$RUN_DIR/init.log" 2>/dev/null >&2; exit 1; }
echo "   piltover=$PILTOVER"

# 3. Base deployments.json (Sepolia + appchain rpc/accounts, piltover, USDC). The
#    appchain account comes from the generated rollup genesis.
echo "→ writing base deployments.json…"
node -e '
  const fs = require("node:fs");
  const g = require(process.argv[1]);
  const [addr, acct] = Object.entries(g.accounts)[0];
  const d = {
    settlement: {
      rpcUrl: process.argv[2], explorer: "https://sepolia.voyager.online",
      torii: "http://localhost:" + process.argv[8],
      account: { address: process.argv[3], privateKey: process.argv[4] },
      piltover: process.argv[5], usdc: process.argv[6],
    },
    appchain: {
      rpcUrl: "http://localhost:" + process.argv[7],
      explorer: "http://localhost:" + process.argv[7] + "/explorer",
      torii: "http://localhost:" + process.argv[9],
      account: { address: addr, privateKey: acct.privateKey },
    },
  };
  fs.writeFileSync(process.argv[10], JSON.stringify(d, null, 2) + "\n");
' "$CHAIN_DIR/genesis.json" "$SEPOLIA_RPC_URL" "$OPERATOR_ADDRESS" "$OPERATOR_PRIVATE_KEY" \
  "$PILTOVER" "$USDC_ADDRESS" "$APPCHAIN_PORT" "$TORII_SCORE_HTTP" "$TORII_GAME_HTTP" \
  "$DEMO_DIR/app/src/deployments.json"

# 4. Appchain rollup node, settling to piltover on Sepolia, L1→L2 messaging on.
echo "→ starting appchain node on :${APPCHAIN_PORT}…"
"$KATANA" --chain "$CHAIN_DIR" --tee mock --dev --dev.no-fee --http.port "$APPCHAIN_PORT" \
  --http.cors_origins '*' --explorer --messaging.enabled \
  > "$RUN_DIR/appchain.log" 2>&1 &
APPCHAIN_PID=$!
until curl -s -o /dev/null "http://localhost:$APPCHAIN_PORT/" 2>/dev/null; do sleep 0.5; done

# 5. saya-tee sidecar: proves appchain blocks, submits update_state to piltover
#    on Sepolia. saya 0.4.0 must be the Poseidon-patched build (see saya-patch).
echo "→ starting saya-tee --mock-prove sidecar (settling to Sepolia)…"
rm -rf "$RUN_DIR/saya-db"
saya-tee tee start --mock-prove \
  --rollup-rpc "http://localhost:$APPCHAIN_PORT" \
  --settlement-rpc "$SEPOLIA_RPC_URL" \
  --settlement-piltover-address "$PILTOVER" \
  --tee-registry-address "$TEE_REGISTRY" \
  --settlement-account-address "$SAYA_ADDRESS" \
  --settlement-account-private-key "$SAYA_PRIVATE_KEY" \
  --prover-private-key 0xdeadbeef \
  --db-dir "$RUN_DIR/saya-db" \
  --batch-size 1 \
  --attestor-poll-interval-ms 1000 \
  > "$RUN_DIR/saya.log" 2>&1 &
SAYA_PID=$!

# 6. Deploy the economy + worlds (GAME_TOKEN, score, game, TokenSale, Entry, grants).
( cd "$DEMO_DIR" && bun run scripts/deploy.ts )

SCORE_WORLD=$(node -e 'console.log(require(process.argv[1]).settlement.scoreWorld)' "$DEMO_DIR/app/src/deployments.json")
GAME_WORLD=$(node -e 'console.log(require(process.argv[1]).appchain.gameWorld)' "$DEMO_DIR/app/src/deployments.json")

# 7. Torii indexers. The score world lives on Sepolia — torii resolves the world's
#    deploy block from the contract, so it won't rescan all of Sepolia. The game
#    world is on the local appchain.
echo "→ starting torii (Sepolia: score world) on :${TORII_SCORE_HTTP}…"
rm -rf "$RUN_DIR/torii-score.db" "$RUN_DIR/torii-game.db"
torii --rpc "$SEPOLIA_RPC_URL" --world "$SCORE_WORLD" \
  --http.port "$TORII_SCORE_HTTP" --grpc.port "$TORII_SCORE_GRPC" \
  --relay.port "$TORII_SCORE_RELAY" --relay.webrtc_port $((TORII_SCORE_RELAY+1)) --relay.websocket_port $((TORII_SCORE_RELAY+2)) \
  --http.cors_origins '*' \
  --db-dir "$RUN_DIR/torii-score.db" > "$RUN_DIR/torii-score.log" 2>&1 &
TORII_SCORE_PID=$!

echo "→ starting torii (appchain: game world) on :${TORII_GAME_HTTP}…"
torii --rpc "http://localhost:$APPCHAIN_PORT" --world "$GAME_WORLD" \
  --http.port "$TORII_GAME_HTTP" --grpc.port "$TORII_GAME_GRPC" \
  --relay.port "$TORII_GAME_RELAY" --relay.webrtc_port $((TORII_GAME_RELAY+1)) --relay.websocket_port $((TORII_GAME_RELAY+2)) \
  --http.cors_origins '*' \
  --db-dir "$RUN_DIR/torii-game.db" > "$RUN_DIR/torii-game.log" 2>&1 &
TORII_GAME_PID=$!
until curl -s -o /dev/null "http://localhost:$TORII_GAME_HTTP/" 2>/dev/null; do sleep 0.5; done

echo ""
echo "✓ Demo is up:"
echo "    settlement     : Starknet Sepolia ($SEPOLIA_RPC_URL)"
echo "    appchain RPC   : http://localhost:$APPCHAIN_PORT   explorer: http://localhost:$APPCHAIN_PORT/explorer"
echo "    saya-tee       : running (.run/saya.log)"
echo "    torii (score)  : http://localhost:$TORII_SCORE_HTTP/sql   (.run/torii-score.log)"
echo "    torii (game)   : http://localhost:$TORII_GAME_HTTP/sql    (.run/torii-game.log)"
# Frontend is HTTPS by default (mkcert); set HTTP=1 to serve plain http.
APP_SCHEME=https; [[ "${HTTP:-}" == "1" ]] && APP_SCHEME=http
echo "    frontend       : $APP_SCHEME://localhost:$FRONTEND_PORT"
echo ""

# 8. Frontend (foreground; Ctrl-C stops everything).
( cd "$DEMO_DIR/app" && exec bun run dev )
