#!/bin/sh
# Launches saya-tee in mock-prove mode, settling L3 blocks onto the user's L2
# via the piltover contract that init-chain deployed.

set -eu

# Pull the piltover address out of the chain config TOML that init-chain
# generated. The toml schema is `[settlement.starknet] core_contract = "0x..."`.
# We use sed with a narrow pattern to keep it simple — no toml parser in the
# runtime image.
config_root="/root/.config/katana"
config_file=$(find "$config_root" -mindepth 2 -maxdepth 2 -name config.toml 2>/dev/null | head -n1 || true)
if [ -z "$config_file" ] || [ ! -f "$config_file" ]; then
    echo "[saya-tee] ERROR: chain config not found under $config_root (init-chain didn't run?)" >&2
    exit 1
fi
PILTOVER_ADDRESS=$(sed -n 's/^core_contract[[:space:]]*=[[:space:]]*"\(0x[0-9a-fA-F]\+\)".*/\1/p' "$config_file" | head -n1)
if [ -z "$PILTOVER_ADDRESS" ]; then
    echo "[saya-tee] ERROR: could not parse core_contract address from $config_file" >&2
    sed -n '/^\[settlement\./,/^\[/p' "$config_file" >&2
    exit 1
fi

# In-container katana service is reachable on the compose network as `katana:5050`.
rollup_rpc="${ROLLUP_RPC:-http://katana:5050}"

mkdir -p /var/lib/saya-tee

echo "[saya-tee] starting (mock-prove, rollup=${rollup_rpc}, settlement=${SETTLEMENT_RPC_URL}, piltover=${PILTOVER_ADDRESS})"

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
