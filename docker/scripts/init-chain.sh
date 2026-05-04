#!/bin/sh
# Generates the L3 rollup chain spec via `katana init rollup --tee`.
#
# `katana init` itself declares + deploys the Piltover Appchain core contract
# on the user's L2, calls `set_program_info` and `set_facts_registry` to wire
# it for the TEE proof system, and writes the chain config to
# /root/.config/katana/chains/$CHAIN_ID.
#
# We pass `--tee --tee-registry-address <addr>` so Piltover's facts-registry
# field is pointed at the IAMDTeeRegistry mock that deploy-contracts.sh put on
# chain. `--settlement-chain` is a URL (the user-supplied L2 RPC) — that
# takes the `init-custom-settlement-chain` path which doesn't need a known
# chain id.
#
# Idempotent: skips init if the chain spec already exists. After init, this
# script also extracts the deployed Piltover address from the generated TOML
# and writes it to /shared/addresses.env so saya-tee can find it.

set -eu

if [ ! -f /shared/addresses.env ]; then
    echo "[init-chain] ERROR: /shared/addresses.env missing (deploy-contracts didn't run?)" >&2
    exit 1
fi

# shellcheck disable=SC1091
. /shared/addresses.env

# `katana init` writes the chain config to /root/.config/katana/<chain_id_as_felt_hex>/.
# We don't know that felt hex up front (it's the cairo-short-string encoding of CHAIN_ID),
# so glob for the single config.toml under the parent dir.
config_root="/root/.config/katana"
config_file=$(find "$config_root" -mindepth 2 -maxdepth 2 -name config.toml 2>/dev/null | head -n1 || true)

if [ -n "$config_file" ] && [ -f "$config_file" ]; then
    echo "[init-chain] chain spec at $config_file already exists — skipping init."
else
    echo "[init-chain] running katana init rollup --tee (id=${CHAIN_ID} settlement=${SETTLEMENT_RPC_URL})..."
    katana init rollup \
        --id "${CHAIN_ID}" \
        --settlement-chain "${SETTLEMENT_RPC_URL}" \
        --settlement-account-address "${SETTLEMENT_ACCOUNT_ADDRESS}" \
        --settlement-account-private-key "${SETTLEMENT_ACCOUNT_PRIVATE_KEY}" \
        --tee \
        --tee-registry-address "${TEE_REGISTRY_ADDRESS}"

    config_file=$(find "$config_root" -mindepth 2 -maxdepth 2 -name config.toml 2>/dev/null | head -n1)
    [ -n "$config_file" ] && [ -f "$config_file" ] || {
        echo "[init-chain] ERROR: chain config not found under $config_root after init" >&2
        exit 1
    }
    echo "[init-chain] chain spec written to $config_file"
fi

# Extract the piltover address from the generated TOML so saya-tee can use it.
# The toml schema is `[settlement.starknet] core_contract = "0x..."`. We use
# sed with a narrow pattern to keep it simple — no toml parser in the
# runtime image.
piltover_address=$(sed -n 's/^core_contract[[:space:]]*=[[:space:]]*"\(0x[0-9a-fA-F]\+\)".*/\1/p' "$config_file" | head -n1)
if [ -z "$piltover_address" ]; then
    echo "[init-chain] ERROR: could not parse core_contract address from $config_file" >&2
    sed -n '/^\[settlement\./,/^\[/p' "$config_file" >&2
    exit 1
fi

# Append PILTOVER_ADDRESS to the shared env file (idempotent — replace any
# prior value to keep restarts clean).
grep -v '^PILTOVER_ADDRESS=' /shared/addresses.env > /shared/addresses.env.new || true
echo "PILTOVER_ADDRESS=$piltover_address" >> /shared/addresses.env.new
mv /shared/addresses.env.new /shared/addresses.env

echo "[init-chain] done. /shared/addresses.env:"
cat /shared/addresses.env
