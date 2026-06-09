#!/usr/bin/env bash
# saya-tee sidecar: proves appchain blocks (--mock-prove) and submits update_state to
# piltover on Sepolia. Needs the appchain node up and the bootstrap done (piltover +
# TEE registry). RESET=1 wipes the saya db to re-sync from scratch (up.sh passes it;
# a standalone restart resumes from the existing db by default).
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_common.sh"

PILTOVER="$(read_piltover)";     [[ -n "$PILTOVER" ]]     || svc_fail "no piltover in $CHAIN_DIR/config.toml — run ./up.sh to bootstrap."
TEE_REGISTRY="$(read_tee_registry)"; [[ -n "$TEE_REGISTRY" ]] || svc_fail "no .run/tee_registry — run ./up.sh to bootstrap."

[[ "${RESET:-}" == "1" ]] && rm -rf "$RUN_DIR/saya-db"
echo "→ saya-tee → piltover $PILTOVER"
exec saya-tee tee start --mock-prove \
  --rollup-rpc "http://localhost:$APPCHAIN_PORT" \
  --settlement-rpc "$SETTLEMENT_RPC_URL" \
  --settlement-piltover-address "$PILTOVER" \
  --tee-registry-address "$TEE_REGISTRY" \
  --settlement-account-address "$SAYA_ADDRESS" \
  --settlement-account-private-key "$SAYA_PRIVATE_KEY" \
  --prover-private-key 0xdeadbeef \
  --db-dir "$RUN_DIR/saya-db" \
  --batch-size "${SAYA_BATCH_SIZE:-1}" \
  --attestor-poll-interval-ms 1000
