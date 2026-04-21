#!/bin/sh
# Launches saya-tee in mock-prove mode, settling L3 blocks onto the user's L2
# via the piltover contract deployed by deploy-contracts.

set -eu

if [ ! -f /shared/addresses.env ]; then
    echo "[saya-tee] ERROR: /shared/addresses.env missing (deploy-contracts didn't run?)" >&2
    exit 1
fi

# shellcheck disable=SC1091
. /shared/addresses.env

# In-container katana service is reachable on the compose network as `katana:5050`.
rollup_rpc="${ROLLUP_RPC:-http://katana:5050}"

mkdir -p /var/lib/saya-tee

echo "[saya-tee] starting (mock-prove, rollup=${rollup_rpc}, settlement=${SETTLEMENT_RPC_URL})"

exec saya-tee tee start \
    --mock-prove \
    --rollup-rpc "$rollup_rpc" \
    --settlement-rpc "$SETTLEMENT_RPC_URL" \
    --settlement-piltover-address "$PILTOVER_ADDRESS" \
    --tee-registry-address "$TEE_REGISTRY_ADDRESS" \
    --settlement-account-address "$SETTLEMENT_ACCOUNT_ADDRESS" \
    --settlement-account-private-key "$SETTLEMENT_ACCOUNT_PRIVATE_KEY" \
    --prover-private-key "0xdeadbeef" \
    --db-dir /var/lib/saya-tee \
    --batch-size "${BATCH_SIZE:-1}"
