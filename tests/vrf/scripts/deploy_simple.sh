#!/usr/bin/env bash
#
# Deploy the Simple VRF test contract using sncast.
#
# Usage:
#   ./scripts/deploy_simple.sh \
#     --rpc-url http://localhost:5050 \
#     --account-address 0x... \
#     --private-key 0x... \
#     --vrf-provider 0x...
#
# Prerequisites:
#   - sncast (starknet-foundry) installed and in PATH
#   - scarb installed (for contract compilation)
#   - The contracts directory must be at ../contracts relative to this script

set -euo pipefail

command -v sncast >/dev/null 2>&1 || { echo "Error: sncast not found. Install starknet-foundry: https://github.com/foundry-rs/starknet-foundry" >&2; exit 1; }

# ── Parse arguments ──────────────────────────────────────────────────────────

RPC_URL=""
ACCOUNT_ADDRESS=""
PRIVATE_KEY=""
VRF_PROVIDER=""

while [[ $# -gt 0 ]]; do
    case $1 in
        --rpc-url)         RPC_URL="$2";          shift 2 ;;
        --account-address) ACCOUNT_ADDRESS="$2";   shift 2 ;;
        --private-key)     PRIVATE_KEY="$2";       shift 2 ;;
        --vrf-provider)    VRF_PROVIDER="$2";      shift 2 ;;
        -h|--help)
            echo "Usage: $0 --rpc-url <URL> --account-address <ADDR> --private-key <KEY> --vrf-provider <ADDR>"
            exit 0 ;;
        *)
            echo "Unknown option: $1" >&2; exit 1 ;;
    esac
done

if [[ -z "$RPC_URL" || -z "$ACCOUNT_ADDRESS" || -z "$PRIVATE_KEY" || -z "$VRF_PROVIDER" ]]; then
    echo "Error: all options are required (--rpc-url, --account-address, --private-key, --vrf-provider)" >&2
    echo "Run with --help for usage." >&2
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONTRACTS_DIR="$SCRIPT_DIR/../contracts"
ACCOUNT_NAME="deployer"
ACCOUNTS_FILE=$(mktemp)

# trap 'rm -f "$ACCOUNTS_FILE"' EXIT

# ── Import account ───────────────────────────────────────────────────────────

echo "Importing account into sncast..."
sncast --accounts-file "$ACCOUNTS_FILE" \
    account import \
    --url "$RPC_URL" \
    --name "$ACCOUNT_NAME" \
    --address "$ACCOUNT_ADDRESS" \
    --private-key "$PRIVATE_KEY" \
    --type oz \
    --silent

# ── Declare ──────────────────────────────────────────────────────────────────

echo "Declaring Simple contract..."
cd "$CONTRACTS_DIR"
DECLARE_OUTPUT=$(sncast --accounts-file "$ACCOUNTS_FILE" --account "$ACCOUNT_NAME" \
    --wait \
    declare \
    --url "$RPC_URL" \
    --contract-name Simple 2>&1) || true

# Parse class hash — handle both fresh declaration and "already declared"
CLASS_HASH=$(echo "$DECLARE_OUTPUT" \
    | sed -n 's/.*[Cc]lass [Hh]ash:[[:space:]]*\(0x[0-9a-fA-F]*\).*/\1/p' \
    | head -1)

# If class hash not in output, check if "already declared" and extract hash from error
if [[ -z "$CLASS_HASH" ]]; then
    CLASS_HASH=$(echo "$DECLARE_OUTPUT" \
        | sed -n 's/.*class hash \(0x[0-9a-fA-F]*\) is already declared.*/\1/p' \
        | head -1)
fi

if [[ -z "$CLASS_HASH" ]]; then
    echo "Failed to parse class hash from declare output:" >&2
    echo "$DECLARE_OUTPUT" >&2
    exit 1
fi

echo "Class hash: $CLASS_HASH"

# ── Deploy ───────────────────────────────────────────────────────────────────

echo "Deploying Simple contract with VRF provider: $VRF_PROVIDER"
DEPLOY_OUTPUT=$(sncast --accounts-file "$ACCOUNTS_FILE" --account "$ACCOUNT_NAME" \
    deploy \
    --url "$RPC_URL" \
    --class-hash "$CLASS_HASH" \
    --constructor-calldata "$VRF_PROVIDER" 2>&1) || {
    echo "Deploy failed:" >&2
    echo "$DEPLOY_OUTPUT" >&2
    exit 1
}

CONTRACT_ADDRESS=$(echo "$DEPLOY_OUTPUT" \
    | sed -n 's/.*[Cc]ontract [Aa]ddress:[[:space:]]*\(0x[0-9a-fA-F]*\).*/\1/p' \
    | head -1)

if [[ -z "$CONTRACT_ADDRESS" ]]; then
    echo "Failed to parse contract address from deploy output:" >&2
    echo "$DEPLOY_OUTPUT" >&2
    exit 1
fi

echo ""
echo "Done."
SEP='+------------------+--------------------------------------------------------------------+'
printf '%s\n' "$SEP"
printf '| %-16s | %-66s |\n' "Class hash"       "$CLASS_HASH"
printf '%s\n' "$SEP"
printf '| %-16s | %-66s |\n' "Contract address" "$CONTRACT_ADDRESS"
printf '%s\n' "$SEP"
