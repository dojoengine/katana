#!/usr/bin/env bash
# Torii indexer for the bank world on the settlement chain (Sepolia). Resolves the
# world's deploy block from the contract, so it doesn't rescan all of Sepolia.
# RESET=1 wipes the db to re-index from scratch — use it when the indexer fell behind
# or skipped events on a flaky RPC (up.sh passes RESET=1; standalone resumes by default).
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_common.sh"

BANK_WORLD="$(read_deployment settlement.bankWorld)" || svc_fail "no settlement.bankWorld in deployments.json — run ./up.sh (deploy step) first."

free_port "$TORII_SCORE_HTTP"; free_port "$TORII_SCORE_GRPC"
for p in "$TORII_SCORE_RELAY" $((TORII_SCORE_RELAY+1)) $((TORII_SCORE_RELAY+2)); do free_port "$p"; done
[[ "${RESET:-}" == "1" ]] && rm -rf "$RUN_DIR/torii-score.db"
echo "→ torii (bank world $BANK_WORLD on $SETTLEMENT_NAME) on :$TORII_SCORE_HTTP"
exec torii --rpc "$SETTLEMENT_RPC_URL" --world "$BANK_WORLD" \
  --http.port "$TORII_SCORE_HTTP" --grpc.port "$TORII_SCORE_GRPC" \
  --relay.port "$TORII_SCORE_RELAY" --relay.webrtc_port $((TORII_SCORE_RELAY+1)) --relay.websocket_port $((TORII_SCORE_RELAY+2)) \
  --http.cors_origins '*' \
  --db-dir "$RUN_DIR/torii-score.db"
