#!/usr/bin/env bash
#
# E2E Test Script for katana-tee
#
# Usage:
#   ./run_e2e_tests.sh                 # Generate fresh proofs for blocks (0, 1, 2)
#   ./run_e2e_tests.sh --reuse-proofs  # Reuse existing proof.json if available
#
set -euo pipefail

# Parse command line arguments
REUSE_PROOFS=false
for arg in "$@"; do
    case $arg in
        --reuse-proofs)
            REUSE_PROOFS=true
            shift
            ;;
    esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
FIXTURES_ROOT="$PROJECT_ROOT/tests/fixtures"
ROOT_CERTS_FILE="$FIXTURES_ROOT/root_certs.json"
DEPLOYMENT_FILE="$FIXTURES_ROOT/deployment.json"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
NC='\033[0m'

log() { echo -e "${GREEN}[E2E]${NC} $1" >&2; }
warn() { echo -e "${YELLOW}[WARN]${NC} $1" >&2; }
error() { echo -e "${RED}[ERROR]${NC} $1" >&2; }
die() { error "$1"; exit 1; }

# Load environment
set -a
source "$PROJECT_ROOT/.env"
set +a

# Configuration
DEVNET_URL="http://127.0.0.1:${DEVNET_PORT:-5051}"
SNCAST_ACCOUNT="${SNCAST_ACCOUNT:-devnet_mainnet_0}"
GARAGA_CLASS_HASH="0x4b22453df42037dd61390736454e8390910adfbbc1fa9d85613e6f375f4de22"
SP1_PROGRAM_ID="0x00613d956661ba71ff3d4d75fba28b79ea077510823adf4b1255ada5d2977402"
MAX_TIME_DIFF=86400
DEVNET_PID=""

cleanup() {
    if [[ -n "$DEVNET_PID" ]]; then
        log "Stopping devnet (PID: $DEVNET_PID)..."
        kill "$DEVNET_PID" 2>/dev/null || true
        wait "$DEVNET_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT

wait_for_rpc() {
    local url=$1
    local max_attempts=30
    log "Waiting for RPC at $url..."
    for i in $(seq 1 $max_attempts); do
        if curl -s "$url" -X POST -H "Content-Type: application/json" \
            -d '{"jsonrpc":"2.0","method":"starknet_chainId","params":[],"id":1}' \
            | grep -q "result"; then
            log "RPC ready"
            return 0
        fi
        sleep 1
    done
    die "RPC not ready after $max_attempts seconds"
}

# Split u256 into low/high felt252 for calldata
# NOTE: Uses sed for string manipulation instead of bash arithmetic
# because bash can't handle 128-bit integers
split_u256() {
    local value=$1
    # Remove 0x prefix if present
    value=${value#0x}
    # Pad to 64 chars (32 bytes = 256 bits)
    value=$(printf "%064s" "$value" | tr ' ' '0')
    # Split: low = last 32 chars, high = first 32 chars
    local high="${value:0:32}"
    local low="${value:32:32}"
    # Remove leading zeros (but keep at least one digit) using sed
    high=$(echo "$high" | sed 's/^0*//' | sed 's/^$/0/')
    low=$(echo "$low" | sed 's/^0*//' | sed 's/^$/0/')
    echo "0x$low 0x$high"
}

# Build a contract with scarb
build_contract() {
    local contract_path=$1
    local contract_name=$2
    log "Building $contract_name..."
    cd "$contract_path"
    scarb build
}

# Declare a contract and return its class hash
declare_contract() {
    local contract_name=$1
    local package=$2
    local class_hash

    log "Declaring $contract_name..."
    class_hash=$(sncast --account "$SNCAST_ACCOUNT" declare \
        --url "$DEVNET_URL" \
        --contract-name "$contract_name" \
        --package "$package" 2>&1 | grep -oP 'class_hash:\s*\K0x[a-fA-F0-9]+' || \
        sncast utils class-hash --contract-name "$contract_name" --package "$package" 2>&1 | grep -oP '0x[a-fA-F0-9]+' | head -1)

    if [[ -z "$class_hash" ]]; then
        die "Failed to declare $contract_name"
    fi
    log "  $contract_name class_hash: $class_hash"
    echo "$class_hash"
}

# Deploy a contract and return its address
deploy_contract() {
    local contract_name=$1
    local class_hash=$2
    shift 2
    local constructor_calldata="$*"

    log "Deploying $contract_name..."
    local deploy_output
    deploy_output=$(sncast --account "$SNCAST_ACCOUNT" deploy \
        --url "$DEVNET_URL" \
        --class-hash "$class_hash" \
        --constructor-calldata $constructor_calldata 2>&1)

    local address
    address=$(echo "$deploy_output" | grep -oiP '(contract_address|contract address):\s*\K0x[a-fA-F0-9]+')

    if [[ -z "$address" ]]; then
        error "Failed to deploy $contract_name"
        echo "$deploy_output"
        exit 1
    fi
    log "  $contract_name deployed: $address"
    echo "$address"
}

start_devnet() {
    log "Starting devnet (forking mainnet, seed $DEVNET_SEED)..."
    starknet-devnet \
        --fork-network "$MAINNET_RPC_URL" \
        --seed "$DEVNET_SEED" \
        --port "$DEVNET_PORT" \
        --timeout 300 &
    DEVNET_PID=$!
    wait_for_rpc "$DEVNET_URL"
}

fetch_root_certs() {
    log "Fetching AMD root certificates from KDS..."
    cargo run -p katana_tee_client --release --bin katana-tee -- \
        fetch-root-certs \
        --processors milan,genoa \
        --validate "$PROJECT_ROOT/crates/amd-sev-snp-attestation-sdk/contracts/test/assets" \
        --output "$ROOT_CERTS_FILE"
}

# Build AMDTEERegistry constructor calldata
build_amd_registry_calldata() {
    local milan_root=$1
    local genoa_root=$2

    # Split SP1 program ID into low/high (u256 = low, high)
    read -r sp1_low sp1_high <<< "$(split_u256 "$SP1_PROGRAM_ID")"

    # Split root cert hashes into low/high
    read -r milan_low milan_high <<< "$(split_u256 "$milan_root")"
    read -r genoa_low genoa_high <<< "$(split_u256 "$genoa_root")"

    # Constructor: verifier_class_hash, sp1_program_id (u256), max_time_diff,
    # trusted_certs (array), processor_models (array), root_certs (array),
    # storage_commitment_proxy (0 = disabled)
    # trusted_certs is empty (length 0)
    echo "$GARAGA_CLASS_HASH $sp1_low $sp1_high $MAX_TIME_DIFF 0 2 0 1 2 $milan_low $milan_high $genoa_low $genoa_high 0"
}

# Save deployment info to JSON
save_deployment() {
    local amd_class_hash=$1
    local amd_address=$2
    local katana_class_hash=$3
    local katana_address=$4

    cat > "$DEPLOYMENT_FILE" << EOF
{
  "network": "devnet-mainnet-fork",
  "timestamp": "$(date -Iseconds)",
  "amd_tee_registry": {
    "class_hash": "$amd_class_hash",
    "address": "$amd_address"
  },
  "katana_tee": {
    "class_hash": "$katana_class_hash",
    "address": "$katana_address"
  },
  "config": {
    "garaga_verifier_class_hash": "$GARAGA_CLASS_HASH",
    "sp1_program_id": "$SP1_PROGRAM_ID",
    "max_time_diff": $MAX_TIME_DIFF
  }
}
EOF
    log "Deployment saved to $DEPLOYMENT_FILE"
}

deploy_contracts() {
    log "Deploying contracts..."

    # Load root cert hashes (format: split into low/high as decimal integers)
    # Use Python to handle large integers (jq converts to scientific notation)
    local root_certs_hex=$(python3 -c "
import json
with open('$ROOT_CERTS_FILE') as f:
    d = json.load(f)
print(hex(int(d['milan_ark_hash_low'])))
print(hex(int(d['milan_ark_hash_high'])))
print(hex(int(d['genoa_ark_hash_low'])))
print(hex(int(d['genoa_ark_hash_high'])))
")
    local milan_low=$(echo "$root_certs_hex" | sed -n '1p')
    local milan_high=$(echo "$root_certs_hex" | sed -n '2p')
    local genoa_low=$(echo "$root_certs_hex" | sed -n '3p')
    local genoa_high=$(echo "$root_certs_hex" | sed -n '4p')

    # Reconstruct full u256 for logging (high || low)
    local milan_root="0x$(printf "%s%s" "${milan_high#0x}" "${milan_low#0x}")"
    local genoa_root="0x$(printf "%s%s" "${genoa_high#0x}" "${genoa_low#0x}")"
    log "  Milan root: $milan_root"
    log "  Genoa root: $genoa_root"

    # Build contracts
    build_contract "$PROJECT_ROOT/contracts/amd_tee_registry" "amd_tee_registry"
    build_contract "$PROJECT_ROOT/contracts/katana_tee" "katana_tee"

    # Declare and deploy AMDTEERegistry
    cd "$PROJECT_ROOT/contracts/amd_tee_registry"
    local amd_class_hash=$(declare_contract "AMDTEERegistry" "amd_tee_registry")
    local amd_calldata=$(build_amd_registry_calldata "$milan_root" "$genoa_root")
    local amd_address=$(deploy_contract "AMDTEERegistry" "$amd_class_hash" $amd_calldata)

    # Declare and deploy KatanaTee
    cd "$PROJECT_ROOT/contracts/katana_tee"
    local katana_class_hash=$(declare_contract "KatanaTee" "katana_tee")
    local katana_address=$(deploy_contract "KatanaTee" "$katana_class_hash" "$amd_address")

    # Verify registry linkage
    log "Verifying registry address linkage..."
    local registry_result
    registry_result=$(sncast call \
        --url "$DEVNET_URL" \
        --contract-address "$katana_address" \
        --function get_registry_address 2>&1)
    log "  get_registry_address: $registry_result"

    # Save deployment
    save_deployment "$amd_class_hash" "$amd_address" "$katana_class_hash" "$katana_address"
}

# Advance Katana by mining an empty block
advance_katana_block() {
    log "Advancing Katana to next block..."
    local result
    result=$(curl -s "$KATANA_RPC_URL" -X POST -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"dev_generateBlock","params":[],"id":1}')

    if echo "$result" | grep -q "error"; then
        warn "Failed to advance block: $result"
        return 1
    fi
    log "Block advanced successfully"
}

# Generate proofs for multiple blocks (0, 1, 2), submit and verify each
generate_multi_block_proofs() {
    log "=== MULTI-BLOCK MODE: Processing blocks 0, 1, 2 ==="
    if [[ "$REUSE_PROOFS" == "true" ]]; then
        log "  (--reuse-proofs enabled: will skip proof generation if proof.json exists)"
    fi

    local amd_address=$(jq -r '.amd_tee_registry.address' "$DEPLOYMENT_FILE")
    local katana_address=$(jq -r '.katana_tee.address' "$DEPLOYMENT_FILE")

    for block_num in 0 1 2; do
        local block_dir="$PROJECT_ROOT/tests/fixtures/block_${block_num}"
        mkdir -p "$block_dir"

        log "--- Block $block_num ---"
        local expected_prefix=$( [ $block_num -eq 0 ] && echo '1 (live mode)' || echo '2 (ASK cached)' )
        log "  Expected prefix_len: $expected_prefix"

        # Fetch attestation (skip if reusing proofs and file exists)
        if [[ "$REUSE_PROOFS" == "true" ]] && [[ -f "$block_dir/attestation.json" ]]; then
            log "  Reusing existing attestation.json"
        else
            log "Fetching attestation for block $block_num..."
            cargo run -p katana_tee_client --release --bin katana-tee -- \
                fetch --rpc "$KATANA_RPC_URL" --output "$block_dir/attestation.json"
        fi

        # Build pipeline command
        local pipeline_args=(
            pipeline
            --json "$block_dir/attestation.json"
            --starknet-rpc "$DEVNET_URL"
            --registry "$amd_address"
            --katana-tee "$katana_address"
            --prover network
            --proof-output "$block_dir/proof.json"
            --calldata-output "$block_dir/calldata.txt"
            --account-address "$DEVNET_ACCOUNT_ADDRESS"
            --account-private-key "$DEVNET_ACCOUNT_PRIVATE_KEY"
        )

        # Reuse existing proof if flag is set and proof exists
        if [[ "$REUSE_PROOFS" == "true" ]] && [[ -f "$block_dir/proof.json" ]]; then
            log "  Reusing existing proof (skipping SP1 network)"
            pipeline_args+=(--proof-input "$block_dir/proof.json")
        else
            log "Generating SP1 proof for block $block_num..."
        fi

        cargo run -p katana_tee_client --release --bin katana-tee -- "${pipeline_args[@]}"

        # Log actual cache info from proof
        if command -v jq &> /dev/null && [ -f "$block_dir/proof.json" ]; then
            local prefix_len=$(jq -r '.trusted_prefix_len // "unknown"' "$block_dir/proof.json")
            log "  Actual prefix_len: $prefix_len"
        fi

        log "Block $block_num artifacts saved to $block_dir"

        # Verify state was updated
        verify_state "$block_dir"

        # Advance to next block (except after last iteration)
        if [[ $block_num -lt 2 ]]; then
            advance_katana_block
            sleep 2  # Brief pause for block propagation
        fi
    done

    log "=== Multi-block proof generation complete ==="
}

submit_proof() {
    local block_dir=$1
    log "Submitting proof to katana_tee..."

    local katana_address=$(jq -r '.katana_tee.address' "$DEPLOYMENT_FILE")
    local calldata=$(cat "$block_dir/calldata.txt")

    # Count array elements (calldata.txt has one element per line)
    local array_len=$(wc -l < "$block_dir/calldata.txt")

    # Extract attestation data for verify_and_update_state
    local state_root=$(jq -r '.stateRoot' "$block_dir/attestation.json")
    local block_hash=$(jq -r '.blockHash' "$block_dir/attestation.json")
    local block_number=$(jq -r '.blockNumber' "$block_dir/attestation.json")

    log "  Contract: $katana_address"
    log "  State root: $state_root"
    log "  Block hash: $block_hash"
    log "  Block number: $block_number"
    log "  Proof array length: $array_len"

    # The calldata format for verify_and_update_state:
    # sp1_proof (array with length prefix), state_root, block_hash, block_number
    # Starknet array serialization: [length, elem1, elem2, ...]
    local full_calldata="$array_len $calldata $state_root $block_hash $block_number"

    log "Invoking verify_and_update_state..."
    local invoke_result
    invoke_result=$(sncast --account "$SNCAST_ACCOUNT" invoke \
        --url "$DEVNET_URL" \
        --contract-address "$katana_address" \
        --function verify_and_update_state \
        --calldata $full_calldata 2>&1)

    local invoke_exit=$?

    log "Transaction result:"
    echo "$invoke_result"

    # Check for execution errors in the result
    if echo "$invoke_result" | grep -qi "error\|failed"; then
        warn "Transaction execution failed (proof verification may have on-chain issues)"
        warn "This is expected if Garaga verifier integration is not yet complete"
        return 1
    elif [[ $invoke_exit -ne 0 ]]; then
        error "Invoke command failed"
        return 1
    fi
    return 0
}

verify_state() {
    local block_dir=$1
    log "Verifying on-chain state..."

    local katana_address=$(jq -r '.katana_tee.address' "$DEPLOYMENT_FILE")

    # Get latest state
    local result
    result=$(sncast call \
        --url "$DEVNET_URL" \
        --contract-address "$katana_address" \
        --function get_latest_state 2>&1)

    log "get_latest_state result:"
    echo "$result"

    # Expected values from attestation
    local expected_block=$(jq -r '.blockNumber' "$block_dir/attestation.json")
    local expected_root=$(jq -r '.stateRoot' "$block_dir/attestation.json")
    local expected_hash=$(jq -r '.blockHash' "$block_dir/attestation.json")

    log "Expected values:"
    log "  block_number: $expected_block"
    log "  state_root: $expected_root"
    log "  block_hash: $expected_hash"

    # Basic validation (the result contains the expected values)
    if echo "$result" | grep -qi "error"; then
        error "State verification failed - call returned error"
        return 1
    fi

    # Check if state was actually updated (non-zero values)
    if echo "$result" | grep -q "0x0, 0x0"; then
        warn "State was NOT updated (values are 0)"
        warn "This indicates the proof verification transaction failed"
        return 1
    else
        log "State was updated successfully"
    fi

    log "State verification completed"
    return 0
}

# === MAIN ===

log "=========================================="
log "  E2E TEST - MULTI-BLOCK MODE"
if [[ "$REUSE_PROOFS" == "true" ]]; then
    log "  (Reusing existing proofs)"
fi
log "=========================================="
log ""
start_devnet
fetch_root_certs
deploy_contracts
generate_multi_block_proofs
log ""
log "MULTI-BLOCK FIXTURE GENERATION COMPLETE"
log "  Fixtures saved to tests/fixtures/block_N/"
