#!/bin/sh
# Declares + deploys the mock TEE registry on the user's L2.
#
# Outputs /shared/addresses.env for downstream services (init-chain,
# saya-tee). Idempotent: skips the deploy if /shared/addresses.env already
# exists — wipe the bootstrap volume (`docker compose down -v`) to force a
# fresh deploy.
#
# Piltover itself is no longer deployed here. `katana init rollup --tee`
# (in init-chain.sh) does the piltover declare + deploy + setup-program +
# set-facts-registry wiring directly, so this script only needs to put the
# TEE registry mock on-chain (the address `katana init` then takes via
# `--tee-registry-address`).
#
# Required env vars (set by compose from .env):
#   SETTLEMENT_RPC_URL, SETTLEMENT_ACCOUNT_ADDRESS,
#   SETTLEMENT_ACCOUNT_PRIVATE_KEY, SETTLEMENT_CHAIN_ID,
#   TEE_REGISTRY_SALT.

set -eu

if [ -f /shared/addresses.env ]; then
    echo "[deploy-contracts] /shared/addresses.env already exists — skipping deploy."
    cat /shared/addresses.env
    exit 0
fi

export SAYA_OPS_OUTPUT=json

echo "[deploy-contracts] declare + deploy mock TEE registry (salt=${TEE_REGISTRY_SALT})..."
tee_json=$(saya-ops core-contract declare-and-deploy-tee-registry-mock \
    --salt "${TEE_REGISTRY_SALT}")
tee_registry_address=$(echo "$tee_json" | jq -er .contract_address)
echo "  tee_registry_address=$tee_registry_address"

cat > /shared/addresses.env <<EOF
TEE_REGISTRY_ADDRESS=$tee_registry_address
EOF

echo "[deploy-contracts] done. /shared/addresses.env:"
cat /shared/addresses.env
