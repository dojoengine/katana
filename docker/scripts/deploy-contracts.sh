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

set -eu

if [ -f /shared/addresses.env ]; then
    echo "[deploy-contracts] /shared/addresses.env already exists — skipping deploy."
    cat /shared/addresses.env
    exit 0
fi

# Scrape the first 0x... hex string from saya-ops output for a given label.
# saya-ops uses env_logger, which writes to stderr; we combine stdout+stderr.
extract_addr() {
    label="$1"
    echo "$2" | grep -m1 "$label" | grep -oE '0x[0-9a-fA-F]+' | head -1
}

extract_block() {
    # saya-ops logs "At block  : Some(N)" on deploy.
    echo "$1" | grep -m1 "At block" | grep -oE '[0-9]+' | head -1
}

echo "[deploy-contracts] 1/4 declare + deploy mock TEE registry (salt=${TEE_REGISTRY_SALT})..."
tee_output=$(saya-ops core-contract declare-and-deploy-tee-registry-mock \
    --salt "${TEE_REGISTRY_SALT}" 2>&1)
echo "$tee_output"
tee_registry_address=$(extract_addr "TEE registry mock address" "$tee_output")
if [ -z "${tee_registry_address:-}" ]; then
    echo "[deploy-contracts] ERROR: could not parse TEE registry address" >&2
    exit 1
fi

echo "[deploy-contracts] 2/4 declare piltover class..."
saya-ops core-contract declare 2>&1

echo "[deploy-contracts] 3/4 deploy piltover (salt=${PILTOVER_SALT})..."
piltover_output=$(saya-ops core-contract deploy --salt "${PILTOVER_SALT}" 2>&1)
echo "$piltover_output"
piltover_address=$(extract_addr "Core contract address" "$piltover_output")
piltover_block=$(extract_block "$piltover_output")
if [ -z "${piltover_address:-}" ]; then
    echo "[deploy-contracts] ERROR: could not parse piltover address" >&2
    exit 1
fi
if [ -z "${piltover_block:-}" ]; then
    echo "[deploy-contracts] WARN: could not parse piltover deploy block, defaulting to 0"
    piltover_block=0
fi

echo "[deploy-contracts] 4/4 setup-program: wire tee_registry as fact_registry on piltover..."
saya-ops core-contract setup-program \
    --core-contract-address "$piltover_address" \
    --fact-registry-address "$tee_registry_address" \
    --chain-id "${CHAIN_ID}" 2>&1

cat > /shared/addresses.env <<EOF
PILTOVER_ADDRESS=$piltover_address
PILTOVER_BLOCK=$piltover_block
TEE_REGISTRY_ADDRESS=$tee_registry_address
EOF

echo "[deploy-contracts] done. /shared/addresses.env:"
cat /shared/addresses.env
