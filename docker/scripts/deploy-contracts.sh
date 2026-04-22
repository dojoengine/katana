#!/bin/sh
# Declares + deploys piltover and the mock TEE registry on the user's L2.
#
# Outputs /shared/addresses.env for downstream services (init-chain, saya-tee).
# Idempotent: skips the deploy if /shared/addresses.env already exists —
# wipe the bootstrap volume (`docker compose down -v`) to force a fresh deploy.
#
# Required env vars (set by compose from .env):
#   SETTLEMENT_RPC_URL, SETTLEMENT_ACCOUNT_ADDRESS,
#   SETTLEMENT_ACCOUNT_PRIVATE_KEY, SETTLEMENT_CHAIN_ID, CHAIN_ID,
#   TEE_REGISTRY_SALT, PILTOVER_SALT.
#
# Uses `saya-ops --output json` (saya PR #63) and jq to parse the structured
# result, instead of scraping info! log lines.

set -eu

if [ -f /shared/addresses.env ]; then
    echo "[deploy-contracts] /shared/addresses.env already exists — skipping deploy."
    cat /shared/addresses.env
    exit 0
fi

export SAYA_OPS_OUTPUT=json

echo "[deploy-contracts] 1/4 declare + deploy mock TEE registry (salt=${TEE_REGISTRY_SALT})..."
tee_json=$(saya-ops core-contract declare-and-deploy-tee-registry-mock \
    --salt "${TEE_REGISTRY_SALT}")
tee_registry_address=$(echo "$tee_json" | jq -er .contract_address)
echo "  tee_registry_address=$tee_registry_address"

echo "[deploy-contracts] 2/4 declare piltover class..."
saya-ops core-contract declare >/dev/null

echo "[deploy-contracts] 3/4 deploy piltover (salt=${PILTOVER_SALT})..."
piltover_json=$(saya-ops core-contract deploy --salt "${PILTOVER_SALT}")
piltover_address=$(echo "$piltover_json" | jq -er .contract_address)
# deployed_block is null on the already-deployed path (TransactionResult::Noop).
# Default to 0 in that case — katana's settlement-contract-deployed-block
# accepts 0 and will backfill from chain tip.
piltover_block=$(echo "$piltover_json" | jq -r '.deployed_block // 0')
echo "  piltover_address=$piltover_address block=$piltover_block"

echo "[deploy-contracts] 4/4 setup-program: wire tee_registry as fact_registry on piltover..."
saya-ops core-contract setup-program \
    --core-contract-address "$piltover_address" \
    --fact-registry-address "$tee_registry_address" \
    --chain-id "${CHAIN_ID}" >/dev/null

cat > /shared/addresses.env <<EOF
PILTOVER_ADDRESS=$piltover_address
PILTOVER_BLOCK=$piltover_block
TEE_REGISTRY_ADDRESS=$tee_registry_address
EOF

echo "[deploy-contracts] done. /shared/addresses.env:"
cat /shared/addresses.env
