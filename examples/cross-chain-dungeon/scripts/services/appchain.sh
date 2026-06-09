#!/usr/bin/env bash
# Appchain Katana node (the local DUNGEON rollup, settling to piltover on Sepolia).
# Controller-capable (paymaster + session middleware) — always on.
# State persists in .run/appchain-db (only the FRESH bootstrap in up.sh wipes it).
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_common.sh"

[[ -f "$CHAIN_DIR/config.toml" ]] || svc_fail "no rollup config at $CHAIN_DIR — run ./up.sh first to bootstrap (piltover + genesis)."

free_port "$APPCHAIN_PORT"
echo "→ appchain node on :$APPCHAIN_PORT (Controller-capable)"
exec "$KATANA" --chain "$CHAIN_DIR" --tee mock --dev --dev.no-fee --block-time 5000 \
  --data-dir "$APPCHAIN_DB" --http.port "$APPCHAIN_PORT" --http.cors_origins '*' \
  --explorer --messaging.enabled $CONTROLLER_FLAGS
