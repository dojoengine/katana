#!/usr/bin/env bash
# Bring up the full cross-chain game store demo (both directions):
#
#   settlement Katana ("L1", SN_SEPOLIA)
#     + piltover core contract        (deployed by `katana init rollup --tee`)
#     + mock TEE registry             (deployed by `saya-ops`)
#   appchain Katana ("L2", rollup, --tee mock) settling to piltover, with its
#     embedded settlement service (the [settlement.runtime] section) proving each
#     block and driving update_state itself — no external saya-tee sidecar.
#   demo contracts (game_minter, achievements, score_registry)
#   React frontend
#
#   L1 -> L2: piltover.send_message_to_appchain -> appchain mint_game
#   L2 -> L1: appchain send_message_to_l1 -> appchain settles -> score_registry consumes
#
# Ctrl-C tears down the settlement/appchain nodes.
set -euo pipefail

DEMO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$DEMO_DIR/../.." && pwd)"
RUN_DIR="$DEMO_DIR/.run"
CHAIN_DIR="$RUN_DIR/chain-config"
mkdir -p "$RUN_DIR"

# Deterministic seed-0 dev accounts on the settlement node.
#   SETTLE = account 0: bootstrap (TEE registry deploy) + the demo's dev-path
#     buy/bank signer. In Controller mode katana also reserves account 0 as the
#     paymaster *relayer* (gas tank = 1, estimate = 2).
#   SAYA   = account 3: piltover operator + saya's update_state submitter. It MUST
#     be distinct from the paymaster's accounts 0/1/2 — otherwise saya and the
#     relayer race for the same nonce and settlement stalls mid-stream (the relayer
#     bumps the nonce under saya, its update_state is rejected, the orchestrator
#     freezes). init rollup and saya-tee must use the SAME account (the operator
#     deploying piltover is the only one allowed to call update_state).
SETTLE_ADDR="0x127fd5f1fe78a71f8bcd1fec63e3fe2f0486b6ecd5c86a0466c3a21fa5cfcec"
SETTLE_PK="0xc5b2fcab997346f3ea1c00b002ecf6f382c5f9c9659a3894eb783c5320f912"
SAYA_ADDR="0x2af9427c5a277474c079a1283c880ee8a6f0f8fbf73ce969c08d88befec1bba"
SAYA_PK="0x1800000000300000180000000000030000000000003006001800006600"
TEE_REGISTRY_SALT="0x7ee"

# ── Preflight ─────────────────────────────────────────────────────────────────
# up.sh is the one-click entry point. It auto-installs the deps it safely can
# (the Dojo toolchain via asdf, the JS deps below, and the katana binary — built
# from this repo if target/debug is empty), and fails fast with the exact command
# for the heavy prerequisites it won't provide: saya-ops and the sibling dojo
# checkout. See README.md "Prerequisites".
DOJO_DIR="$REPO_ROOT/../dojo"
fail() { echo "error: $1" >&2; exit 1; }

echo "→ preflight…"
# Dojo toolchain (sozo/torii/scarb), pinned in .tool-versions. Idempotent —
# installs only what's missing. Best-effort: the command checks below are the
# real gate, so a hiccup here (e.g. an asdf plugin not added) doesn't abort.
if command -v asdf >/dev/null 2>&1; then
  ( cd "$DEMO_DIR" && asdf install ) || echo "  warning: 'asdf install' had issues; verifying tools below…" >&2
else
  echo "  warning: asdf not found — install it, or put sozo/torii/scarb on PATH (see .tool-versions)." >&2
fi

# katana — always the binary built from THIS repo, never the asdf/PATH katana
# (a different, released version that doesn't have the embedded settlement
# service). Use the existing target/debug build; build it from source if absent.
KATANA="$REPO_ROOT/target/debug/katana"
if [[ ! -x "$KATANA" ]]; then
  echo "  katana not found at target/debug — building from source (cargo build -p katana)…"
  ( cd "$REPO_ROOT" && cargo build -p katana --bin katana ) \
    || fail "failed to build katana. Build it manually:  ( cd \"$REPO_ROOT\" && cargo build -p katana --bin katana )"
fi
[[ -x "$KATANA" ]] || fail "katana still not found at $KATANA after build."

# saya-ops — used once to deploy the mock TEE registry (a bootstrap helper, not
# the settlement sidecar). Settlement itself is now done by katana's embedded
# settlement service (this branch), so saya-tee is no longer required.
command -v saya-ops >/dev/null 2>&1 || fail "'saya-ops' not found on PATH. Install saya v0.4.0 — see ./saya-patch/README.md."

# Dojo toolchain + the sibling dojo checkout the cairo packages depend on by path.
for bin in sozo torii scarb; do
  command -v "$bin" >/dev/null 2>&1 || fail "'$bin' not found on PATH. Run 'asdf install' in this directory (see .tool-versions)."
done
[[ -d "$DOJO_DIR/crates/dojo/core" ]] || fail "dojo checkout not found at $DOJO_DIR — the cairo packages depend on it by path. Clone it as a sibling of katana:  ( cd \"$REPO_ROOT/..\" && git clone https://github.com/dojoengine/dojo )  then check out the sozo-matching ref (sozo/v1.8.7)."
echo "→ katana: $KATANA"
echo "→ saya-ops: $(command -v saya-ops)"
echo "→ sozo: $(sozo --version 2>&1 | head -1)   torii: $(torii --version 2>&1 | head -1)"

# Optional: Cartridge Controller wallet. The default run uses the dev account only
# (no paymaster, fully offline). With `CONTROLLER=1 ./up.sh` both nodes are started
# Controller-capable (settlement for buy/bank, appchain for roll) so the same
# Controller signs on both chains. See README → "Using Controller (optional)".
CONTROLLER_FLAGS=""        # run-node flags: enable the cartridge middleware + paymaster
if [[ "${CONTROLLER:-}" == "1" ]]; then
  CONTROLLER_FLAGS="--paymaster --cartridge.paymaster --cartridge.controllers"
  command -v paymaster-service >/dev/null 2>&1 \
    || echo "  note: 'paymaster-service' not on PATH — katana will try to fetch it (cartridge-gg/paymaster); see docs/cartridge.md." >&2
  echo "→ Controller mode ON: both nodes Controller-capable. Needs a Controller login + (Chrome) the local-network-access flag."
fi

# Torii ports (settlement indexes the score world + piltover; appchain indexes
# the game world). Relay ports must be distinct per instance; chosen away from
# the 8080/9090 defaults to avoid clashing with other local dojo projects.
TORII_SCORE_HTTP=8081; TORII_SCORE_GRPC=50081; TORII_SCORE_RELAY=9181
TORII_GAME_HTTP=8082;  TORII_GAME_GRPC=50082;  TORII_GAME_RELAY=9184

SETTLEMENT_PID=""; APPCHAIN_PID=""; TORII_SCORE_PID=""; TORII_GAME_PID=""
cleanup() {
  echo ""; echo "→ shutting down…"
  [[ -n "$TORII_GAME_PID" ]] && kill "$TORII_GAME_PID" 2>/dev/null || true
  [[ -n "$TORII_SCORE_PID" ]] && kill "$TORII_SCORE_PID" 2>/dev/null || true
  [[ -n "$APPCHAIN_PID" ]] && kill "$APPCHAIN_PID" 2>/dev/null || true
  [[ -n "$SETTLEMENT_PID" ]] && kill "$SETTLEMENT_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

echo "→ installing JS dependencies…"
( cd "$DEMO_DIR" && bun install >/dev/null )
( cd "$DEMO_DIR/app" && bun install >/dev/null )
# Contracts are built + migrated per-world by scripts/deploy.ts (sozo build/migrate).

# 1. Settlement node (SN_SEPOLIA so saya-ops / init rollup chain id match).
echo "→ starting settlement node on :5050…"
"$KATANA" --dev --dev.no-fee --chain-id SN_SEPOLIA --http.port 5050 \
  --http.cors_origins '*' --explorer $CONTROLLER_FLAGS > "$RUN_DIR/settlement.log" 2>&1 &
SETTLEMENT_PID=$!
until curl -s -o /dev/null http://localhost:5050/ 2>/dev/null; do sleep 0.5; done

# 2. Mock TEE registry on settlement (permissive attestation verifier).
#    Deploy from the SAYA account (3), not account 0: in Controller mode the
#    settlement node's paymaster bootstrap deploys the AVNU forwarder from account
#    0 right after startup, and a concurrent deploy from account 0 here races its
#    nonce ("Account nonce: 0x2; got: 0x1") and aborts the launch.
echo "→ deploying mock TEE registry (saya-ops)…"
REG_OUT=$(SETTLEMENT_RPC_URL=http://localhost:5050 \
  SETTLEMENT_ACCOUNT_ADDRESS="$SAYA_ADDR" \
  SETTLEMENT_ACCOUNT_PRIVATE_KEY="$SAYA_PK" \
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
  --settlement-account-address "$SAYA_ADDR" \
  --settlement-account-private-key "$SAYA_PK" \
  --tee \
  --tee-registry-address "$TEE_REGISTRY" \
  --output-path "$CHAIN_DIR" > "$RUN_DIR/init.log" 2>&1
PILTOVER=$(sed -nE 's/^core_contract = "(0x[0-9a-fA-F]+)".*/\1/p' "$CHAIN_DIR/config.toml")
[[ -n "$PILTOVER" ]] || { echo "error: could not parse piltover address from config.toml" >&2; cat "$RUN_DIR/init.log" >&2; exit 1; }
echo "   piltover=$PILTOVER"

# Enable katana's embedded settlement service. `init rollup` writes only the
# settlement *layer* (where to settle); the operator adds the [settlement.runtime]
# section (the settling account + key, TEE registry, batching) that turns this node
# into an active settler. With it present, katana proves and submits update_state
# itself — this is the job that used to belong to the saya-tee sidecar.
echo "→ adding [settlement.runtime] (embedded settlement, replaces saya-tee)…"
cat >> "$CHAIN_DIR/config.toml" <<EOF

[settlement.runtime]
account-address = "$SAYA_ADDR"
account-private-key = "$SAYA_PK"
tee-registry = "$TEE_REGISTRY"
batch-size = 1
EOF

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
      torii: "http://localhost:" + process.argv[6],
      account: { address: process.argv[2], privateKey: process.argv[3] },
      piltover: process.argv[4],
    },
    appchain: {
      rpcUrl: "http://localhost:5051", explorer: "http://localhost:5051/explorer",
      torii: "http://localhost:" + process.argv[7],
      account: { address: addr, privateKey: acct.privateKey },
    },
  };
  fs.writeFileSync(process.argv[5], JSON.stringify(d, null, 2) + "\n");
' "$CHAIN_DIR/genesis.json" "$SETTLE_ADDR" "$SETTLE_PK" "$PILTOVER" "$DEMO_DIR/app/src/deployments.json" "$TORII_SCORE_HTTP" "$TORII_GAME_HTTP"

# 5. Appchain rollup node, settling to piltover, with L1->L2 messaging enabled.
echo "→ starting appchain node on :5051…"
# --dev --dev.no-fee disables fees on the rollup (mirrors the saya-tee test
# harness' fee:false config); without it, fee estimation on the near-empty
# rollup produces resource bounds below the actual gas price and txs revert.
# --block-time 5000 mines a block every 5s (interval mining) instead of per-tx, so
# the chain advances steadily and saya keeps settling even when the app is idle.
"$KATANA" --chain "$CHAIN_DIR" --tee mock --dev --dev.no-fee --block-time 5000 --http.port 5051 \
  --http.cors_origins '*' --explorer --messaging.enabled $CONTROLLER_FLAGS \
  > "$RUN_DIR/appchain.log" 2>&1 &
APPCHAIN_PID=$!
until curl -s -o /dev/null http://localhost:5051/ 2>/dev/null; do sleep 0.5; done

# 6. Settlement: handled by the appchain node's embedded settlement service
#    (configured via the [settlement.runtime] section written above). It proves
#    each appchain block (--tee mock) and submits update_state — carrying the
#    L2->L1 message hashes — to the piltover core. No external saya-tee sidecar.

# 7. Migrate the two Dojo worlds (sozo) and fill in their addresses.
( cd "$DEMO_DIR" && bun run scripts/deploy.ts )

# 7b. (CONTROLLER mode) Declare the Controller account class on the appchain.
#     Workaround for katana #584: `katana init rollup` round-trips genesis.json,
#     shifting the embedded controller class hash, so the canonical class the hosted
#     keychain deploys isn't present after boot. Declaring the on-disk artifact here
#     lands the canonical hash so the Controller can auto-deploy on the appchain.
if [[ "${CONTROLLER:-}" == "1" ]]; then
  echo "→ declaring Controller account class on the appchain (katana #584 workaround)…"
  ( cd "$DEMO_DIR" && bun run scripts/declare-controller-class.ts )
fi

# Read back the migrated world addresses for torii.
SCORE_WORLD=$(node -e 'console.log(require(process.argv[1]).settlement.scoreWorld)' "$DEMO_DIR/app/src/deployments.json")
GAME_WORLD=$(node -e 'console.log(require(process.argv[1]).appchain.gameWorld)' "$DEMO_DIR/app/src/deployments.json")

# 8. Torii indexers — one per chain (a torii instance indexes a single RPC).
#    Each indexes its world's models + events. Purchases sent on L1 (piltover
#    `MessageSent`) are read straight from the settlement RPC by the frontend.
echo "→ starting torii (settlement: score world) on :${TORII_SCORE_HTTP}…"
rm -rf "$RUN_DIR/torii-score.db" "$RUN_DIR/torii-game.db"
torii --rpc http://localhost:5050 --world "$SCORE_WORLD" \
  --http.port "$TORII_SCORE_HTTP" --grpc.port "$TORII_SCORE_GRPC" \
  --relay.port "$TORII_SCORE_RELAY" --relay.webrtc_port $((TORII_SCORE_RELAY+1)) --relay.websocket_port $((TORII_SCORE_RELAY+2)) \
  --http.cors_origins '*' \
  --db-dir "$RUN_DIR/torii-score.db" > "$RUN_DIR/torii-score.log" 2>&1 &
TORII_SCORE_PID=$!

echo "→ starting torii (appchain: game world) on :${TORII_GAME_HTTP}…"
torii --rpc http://localhost:5051 --world "$GAME_WORLD" \
  --http.port "$TORII_GAME_HTTP" --grpc.port "$TORII_GAME_GRPC" \
  --relay.port "$TORII_GAME_RELAY" --relay.webrtc_port $((TORII_GAME_RELAY+1)) --relay.websocket_port $((TORII_GAME_RELAY+2)) \
  --http.cors_origins '*' \
  --db-dir "$RUN_DIR/torii-game.db" > "$RUN_DIR/torii-game.log" 2>&1 &
TORII_GAME_PID=$!
until curl -s -o /dev/null "http://localhost:$TORII_GAME_HTTP/" 2>/dev/null; do sleep 0.5; done

echo ""
echo "✓ Demo is up:"
echo "    settlement RPC : http://localhost:5050   explorer: http://localhost:5050/explorer"
echo "    appchain RPC   : http://localhost:5051   explorer: http://localhost:5051/explorer  (embedded settlement)"
echo "    torii (score)  : http://localhost:$TORII_SCORE_HTTP/sql   (.run/torii-score.log)"
echo "    torii (game)   : http://localhost:$TORII_GAME_HTTP/sql    (.run/torii-game.log)"
# Controller mode serves the app over trusted HTTPS (mkcert) so passkey login works.
APP_URL="http://localhost:3001"; [[ "${CONTROLLER:-}" == "1" ]] && APP_URL="https://localhost:3001"
echo "    frontend       : $APP_URL"
echo ""

# 8. Frontend (foreground; Ctrl-C stops everything). vite.config switches to
#    https when CONTROLLER=1 (inherited from the environment here).
( cd "$DEMO_DIR/app" && exec bun run dev )
