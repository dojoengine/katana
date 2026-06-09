#!/usr/bin/env bash
# Shared config for the per-service launch scripts (scripts/services/*.sh).
#
# Each service script sources this, then frees its port and `exec`s its service in the
# FOREGROUND — so you can start/restart any one service on its own (e.g. re-index a
# torii: `RESET=1 scripts/services/torii-bank.sh`) without the whole stack. `up.sh`
# runs the one-time bootstrap/deploy and then backgrounds these same scripts, so each
# service has a single source of truth for its command.
#
# Prereqs: the bootstrap (piltover + rollup genesis) and deploy must already have run —
# i.e. `up.sh` at least once. The scripts read what they need from .env,
# .run/chain-config, .run/tee_registry, and app/src/deployments.json.
set -euo pipefail

DEMO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
REPO_ROOT="$(cd "$DEMO_DIR/../.." && pwd)"
RUN_DIR="$DEMO_DIR/.run"
CHAIN_DIR="$RUN_DIR/chain-config"
APPCHAIN_DB="$RUN_DIR/appchain-db"   # persistent appchain state — survives restarts
DEPLOYMENTS="$DEMO_DIR/app/src/deployments.json"

# Ports — keep in sync with up.sh.
APPCHAIN_PORT=5070
TORII_SCORE_HTTP=8091; TORII_SCORE_GRPC=50091; TORII_SCORE_RELAY=9191
TORII_GAME_HTTP=8092;  TORII_GAME_GRPC=50092;  TORII_GAME_RELAY=9194
FRONTEND_PORT=3002

svc_fail() { echo "error: $1" >&2; exit 1; }

# .env → operator/saya accounts, USDC, settlement RPC.
[[ -f "$DEMO_DIR/.env" ]] || svc_fail "no .env — copy .env.example to .env (see up.sh)."
set -a; # shellcheck disable=SC1091
source "$DEMO_DIR/.env"; set +a

# Settlement network derivation (mirrors up.sh).
SETTLEMENT_NETWORK="${SETTLEMENT_NETWORK:-sepolia}"
SETTLEMENT_RPC_URL="${SETTLEMENT_RPC_URL:-${SEPOLIA_RPC_URL:-}}"
case "$SETTLEMENT_NETWORK" in
  sepolia) SETTLEMENT_CHAIN_ID="SN_SEPOLIA"; SETTLEMENT_EXPLORER="https://sepolia.voyager.online"; SETTLEMENT_NAME="Starknet Sepolia" ;;
  mainnet) SETTLEMENT_CHAIN_ID="SN_MAIN";    SETTLEMENT_EXPLORER="https://voyager.online";          SETTLEMENT_NAME="Starknet Mainnet" ;;
  *) svc_fail "SETTLEMENT_NETWORK must be 'sepolia' or 'mainnet' (got '$SETTLEMENT_NETWORK')." ;;
esac
[[ -n "${SETTLEMENT_RPC_URL:-}" ]] || svc_fail "set SETTLEMENT_RPC_URL (or SEPOLIA_RPC_URL) in .env."

# katana binary (release > debug > PATH).
if   [[ -x "$REPO_ROOT/target/release/katana" ]]; then KATANA="$REPO_ROOT/target/release/katana"
elif [[ -x "$REPO_ROOT/target/debug/katana"   ]]; then KATANA="$REPO_ROOT/target/debug/katana"
elif command -v katana >/dev/null 2>&1;            then KATANA="$(command -v katana)"
else svc_fail "katana binary not found — build it: ( cd \"$REPO_ROOT\" && cargo build --release )"; fi

# CONTROLLER=1 → appchain paymaster/session middleware + Controller auto-deploy.
CONTROLLER_FLAGS=""
[[ "${CONTROLLER:-}" == "1" ]] && CONTROLLER_FLAGS="--paymaster --cartridge.paymaster --cartridge.controllers"

# Kill whatever holds a TCP port so re-running a service script restarts it cleanly.
free_port() {
  local pids; pids="$(lsof -ti "tcp:$1" 2>/dev/null || true)"
  [[ -n "$pids" ]] && kill -9 $pids 2>/dev/null || true
}

# Read a dotted key out of deployments.json (e.g. read_deployment settlement.bankWorld).
read_deployment() {
  node -e 'let v=require(process.argv[2]);for(const k of process.argv[1].split("."))v=v?.[k];if(v==null)process.exit(3);console.log(v)' "$1" "$DEPLOYMENTS" 2>/dev/null
}
# piltover from the generated rollup config; TEE registry from the bootstrap file.
read_piltover()     { sed -nE 's/^core_contract = "(0x[0-9a-fA-F]+)".*/\1/p' "$CHAIN_DIR/config.toml" 2>/dev/null; }
read_tee_registry() { cat "$RUN_DIR/tee_registry" 2>/dev/null; }
