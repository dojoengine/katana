#!/bin/sh
# Generates the L3 rollup chain spec via `katana init rollup --tee`.
#
# `katana init` itself declares + deploys the Piltover Appchain core contract
# on the user's L2, calls `set_program_info` and `set_facts_registry` to wire
# it for the TEE proof system, and writes the chain config to
# /root/.config/katana/<chain_id_as_felt_hex>/config.toml.
#
# We pass `--tee --tee-registry-address <addr>` so Piltover's facts-registry
# field is pointed at the user-supplied IAMDTeeRegistry mock. The user is
# responsible for deploying that mock on their L2 before bringing this
# compose up — see README for one-line instructions.
#
# Idempotent: skips init if the chain spec already exists.

set -eu

# `katana init` writes the chain config to /root/.config/katana/<chain_id_as_felt_hex>/.
# We don't know that felt hex up front (it's the cairo-short-string encoding of CHAIN_ID),
# so glob for the single config.toml under the parent dir.
config_root="/root/.config/katana"
config_file=$(find "$config_root" -mindepth 2 -maxdepth 2 -name config.toml 2>/dev/null | head -n1 || true)

if [ -n "$config_file" ] && [ -f "$config_file" ]; then
    echo "[init-chain] chain spec at $config_file already exists — skipping init."
    exit 0
fi

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
