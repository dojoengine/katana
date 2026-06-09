#!/usr/bin/env bash
# Torii indexer for the game world on the local appchain. --indexing.preconfirmed
# indexes the pre-confirmed (pending) block so a play action's model writes appear
# immediately instead of waiting for the 5s block tick. RESET=1 wipes the db to
# re-index (up.sh passes it; standalone resumes by default).
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_common.sh"

GAME_WORLD="$(read_deployment appchain.gameWorld)" || svc_fail "no appchain.gameWorld in deployments.json — run ./up.sh (deploy step) first."

free_port "$TORII_GAME_HTTP"; free_port "$TORII_GAME_GRPC"
for p in "$TORII_GAME_RELAY" $((TORII_GAME_RELAY+1)) $((TORII_GAME_RELAY+2)); do free_port "$p"; done
[[ "${RESET:-}" == "1" ]] && rm -rf "$RUN_DIR/torii-game.db"
echo "→ torii (game world $GAME_WORLD on appchain) on :$TORII_GAME_HTTP"
exec torii --rpc "http://localhost:$APPCHAIN_PORT" --world "$GAME_WORLD" \
  --http.port "$TORII_GAME_HTTP" --grpc.port "$TORII_GAME_GRPC" \
  --relay.port "$TORII_GAME_RELAY" --relay.webrtc_port $((TORII_GAME_RELAY+1)) --relay.websocket_port $((TORII_GAME_RELAY+2)) \
  --http.cors_origins '*' --indexing.preconfirmed \
  --db-dir "$RUN_DIR/torii-game.db"
