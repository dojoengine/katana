#!/bin/sh
# Generates the L3 rollup chain spec via `katana init rollup`, using the
# piltover + tee_registry addresses produced by deploy-contracts.
#
# The settlement chain is passed as a URL (SETTLEMENT_RPC_URL) rather than a
# shortname like "sepolia". Passing a URL requires --settlement-facts-registry,
# which we set to the TEE registry mock address — correct for every L2 in
# TEE-mock mode, where the TEE registry doubles as piltover's fact registry.
#
# Idempotent: skips init if the chain spec already exists under
# /root/.config/katana/chains/$CHAIN_ID.

set -eu

if [ ! -f /shared/addresses.env ]; then
    echo "[init-chain] ERROR: /shared/addresses.env missing (deploy-contracts didn't run?)" >&2
    exit 1
fi

# shellcheck disable=SC1091
. /shared/addresses.env

chain_dir="/root/.config/katana/chains/${CHAIN_ID}"
if [ -d "$chain_dir" ]; then
    echo "[init-chain] chain spec at $chain_dir already exists — skipping init."
    exit 0
fi

echo "[init-chain] running katana init rollup (id=${CHAIN_ID} settlement=${SETTLEMENT_RPC_URL})..."
katana init rollup \
    --id "${CHAIN_ID}" \
    --settlement-chain "${SETTLEMENT_RPC_URL}" \
    --settlement-facts-registry "${TEE_REGISTRY_ADDRESS}" \
    --settlement-contract "${PILTOVER_ADDRESS}" \
    --settlement-contract-deployed-block "${PILTOVER_BLOCK}" \
    --settlement-account-address "${SETTLEMENT_ACCOUNT_ADDRESS}" \
    --settlement-account-private-key "${SETTLEMENT_ACCOUNT_PRIVATE_KEY}"

echo "[init-chain] done. Chain spec written to $chain_dir"
