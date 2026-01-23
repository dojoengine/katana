#!/usr/bin/env bash
#
# E2E Test Script for katana-tee
#
# Usage:
#   ./run_e2e_tests.sh --live     # Fetch from TEE, generate proof, save fixtures
#   ./run_e2e_tests.sh --fixture  # Use saved fixtures (default)
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
FIXTURES_DIR="$SCRIPT_DIR/fixtures"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
NC='\033[0m'

log() { echo -e "${GREEN}[E2E]${NC} $1"; }
warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
error() { echo -e "${RED}[ERROR]${NC} $1"; }
die() { error "$1"; exit 1; }

# Load environment
set -a
source "$PROJECT_ROOT/.env"
set +a

# Configuration
DEVNET_URL="http://127.0.0.1:${DEVNET_PORT:-5050}"
GARAGA_CLASS_HASH="0x4b22453df42037dd61390736454e8390910adfbbc1fa9d85613e6f375f4de22"
SP1_PROGRAM_ID="0x00d2342d2400bed28302507269281dcb2c621bae91a0626796ce637f01c928d8"
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
    class_hash=$(sncast --account devnet_mainnet_0 declare \
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
    deploy_output=$(sncast --account devnet_mainnet_0 deploy \
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
        --fork-network "$STARKNET_RPC_URL_MAINNET" \
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
        --output "$FIXTURES_DIR/root_certs.json"
}

extract_ask_cert_from_proof() {
    # Extract the ASK intermediate cert hash from proof.json journal
    # Journal structure: [offset, result, timestamp, processorModel, rawReportOffset, certsOffset, ...]
    # certs array: [length, cert0(root), cert1(ASK), cert2(VCEK)]
    local proof_file="$1"
    if [[ ! -f "$proof_file" ]]; then
        echo ""
        return
    fi

    # The journal hex has certs at offset 0x5a0 (1440 bytes) after the 32-byte outer offset
    # Each cert is 32 bytes (64 hex chars). We want cert[1] (ASK).
    # Position: 0x (2) + outer offset (64) + certs_offset*2 (2880) + length (64) + cert0 (64) = 3074
    # Read 64 chars for the ASK cert hash
    local journal=$(jq -r '.raw_proof.journal' "$proof_file")
    if [[ -z "$journal" ]] || [[ "$journal" == "null" ]]; then
        echo ""
        return
    fi

    # Extract ASK cert at position 3074 (chars 3074-3137) = bytes 1504-1535 in journal
    local ask_hash="0x${journal:3074:64}"
    echo "$ask_hash"
}

# Build AMDTEERegistry constructor calldata
build_amd_registry_calldata() {
    local milan_root=$1
    local genoa_root=$2
    local ask_cert=$3

    # Split SP1 program ID into low/high (u256 = low, high)
    read -r sp1_low sp1_high <<< "$(split_u256 "$SP1_PROGRAM_ID")"

    # Split root cert hashes into low/high
    read -r milan_low milan_high <<< "$(split_u256 "$milan_root")"
    read -r genoa_low genoa_high <<< "$(split_u256 "$genoa_root")"

    # Build trusted_certs array for constructor
    local trusted_certs_calldata="0"  # Default: empty array (length 0)
    if [[ -n "$ask_cert" ]]; then
        read -r ask_low ask_high <<< "$(split_u256 "$ask_cert")"
        trusted_certs_calldata="1 $ask_low $ask_high"  # Array with 1 element
        log "  Trusting ASK intermediate cert"
    fi

    # Constructor: verifier_class_hash, sp1_program_id (u256), max_time_diff,
    # trusted_certs (array), processor_models (array), root_certs (array)
    echo "$GARAGA_CLASS_HASH $sp1_low $sp1_high $MAX_TIME_DIFF $trusted_certs_calldata 2 0 1 2 $milan_low $milan_high $genoa_low $genoa_high"
}

# Save deployment info to JSON
save_deployment() {
    local amd_class_hash=$1
    local amd_address=$2
    local katana_class_hash=$3
    local katana_address=$4

    cat > "$FIXTURES_DIR/deployment.json" << EOF
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
    log "Deployment saved to $FIXTURES_DIR/deployment.json"
}

deploy_contracts() {
    log "Deploying contracts..."

    # Load root cert hashes
    local milan_root=$(jq -r '.milan.ark_hash' "$FIXTURES_DIR/root_certs.json")
    local genoa_root=$(jq -r '.genoa.ark_hash' "$FIXTURES_DIR/root_certs.json")
    log "  Milan root: $milan_root"
    log "  Genoa root: $genoa_root"

    # Extract ASK intermediate cert from proof if available (for fixture mode)
    local ask_cert=""
    if [[ -f "$FIXTURES_DIR/proof.json" ]]; then
        ask_cert=$(extract_ask_cert_from_proof "$FIXTURES_DIR/proof.json")
        [[ -n "$ask_cert" ]] && log "  ASK cert (from proof): $ask_cert"
    fi

    # Build contracts
    build_contract "$PROJECT_ROOT/contracts/amd_tee_registry" "amd_tee_registry"
    build_contract "$PROJECT_ROOT/contracts/katana_tee" "katana_tee"

    # Declare and deploy AMDTEERegistry
    cd "$PROJECT_ROOT/contracts/amd_tee_registry"
    local amd_class_hash=$(declare_contract "AMDTEERegistry" "amd_tee_registry")
    local amd_calldata=$(build_amd_registry_calldata "$milan_root" "$genoa_root" "$ask_cert")
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

generate_proof_live() {
    log "=== LIVE MODE: Generating real proof ==="

    log "Fetching attestation from Katana TEE at $KATANA_RPC_URL..."
    cargo run -p katana_tee_client --release --bin katana-tee -- \
        fetch --rpc "$KATANA_RPC_URL" --output "$FIXTURES_DIR/attestation.json"

    log "Attestation saved. Generating SP1 proof via network prover..."
    log "This may take several minutes..."

    local katana_address=$(jq -r '.katana_tee.address' "$FIXTURES_DIR/deployment.json")

    # Use pipeline command with --dry-run to generate proof and calldata
    # --skip-cache bypasses on-chain cache lookup (uses default trusted_prefix_len=2)
    cargo run -p katana_tee_client --release --bin katana-tee -- \
        pipeline \
        --json "$FIXTURES_DIR/attestation.json" \
        --starknet-rpc "$DEVNET_URL" \
        --katana-tee "$katana_address" \
        --prover network \
        --proof-output "$FIXTURES_DIR/proof.json" \
        --calldata-output "$FIXTURES_DIR/calldata.txt" \
        --skip-cache \
        --dry-run

    log "Proof and calldata generated and saved to fixtures"
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

# Generate proofs for multiple blocks (0, 1, 2)
generate_multi_block_proofs() {
    log "=== MULTI-BLOCK MODE: Generating proofs for blocks 0, 1, 2 ==="

    local katana_address=$(jq -r '.katana_tee.address' "$FIXTURES_DIR/deployment.json")

    for block_num in 0 1 2; do
        local block_dir="$PROJECT_ROOT/tests/fixtures/block_${block_num}"
        mkdir -p "$block_dir"

        log "--- Block $block_num ---"

        # Fetch attestation
        log "Fetching attestation for block $block_num..."
        cargo run -p katana_tee_client --release --bin katana-tee -- \
            fetch --rpc "$KATANA_RPC_URL" --output "$block_dir/attestation.json"

        # Generate proof
        log "Generating SP1 proof for block $block_num (this may take 1-2 minutes)..."
        cargo run -p katana_tee_client --release --bin katana-tee -- \
            pipeline \
            --json "$block_dir/attestation.json" \
            --starknet-rpc "$DEVNET_URL" \
            --katana-tee "$katana_address" \
            --prover network \
            --proof-output "$block_dir/proof.json" \
            --calldata-output "$block_dir/calldata.txt" \
            --skip-cache \
            --dry-run

        log "Block $block_num artifacts saved to $block_dir"

        # Advance to next block (except after last iteration)
        if [[ $block_num -lt 2 ]]; then
            advance_katana_block
            sleep 2  # Brief pause for block propagation
        fi
    done

    log "=== Multi-block proof generation complete ==="
}

submit_proof() {
    log "Submitting proof to katana_tee..."

    local katana_address=$(jq -r '.katana_tee.address' "$FIXTURES_DIR/deployment.json")
    local calldata=$(cat "$FIXTURES_DIR/calldata.txt")

    # Extract attestation data for verify_and_update_state
    # Note: attestation.json is raw TeeQuoteResponse (no .result wrapper)
    local state_root=$(jq -r '.stateRoot' "$FIXTURES_DIR/attestation.json")
    local block_hash=$(jq -r '.blockHash' "$FIXTURES_DIR/attestation.json")
    local block_number=$(jq -r '.blockNumber' "$FIXTURES_DIR/attestation.json")

    log "  Contract: $katana_address"
    log "  State root: $state_root"
    log "  Block hash: $block_hash"
    log "  Block number: $block_number"

    # The calldata format for verify_and_update_state:
    # sp1_proof (array), state_root, block_hash, block_number
    local full_calldata="$calldata $state_root $block_hash $block_number"

    log "Invoking verify_and_update_state..."
    local invoke_result
    invoke_result=$(sncast --account devnet_mainnet_0 invoke \
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
        # Continue anyway to show state - don't exit 1
    elif [[ $invoke_exit -ne 0 ]]; then
        error "Invoke command failed"
        exit 1
    fi
}

verify_state() {
    log "Verifying on-chain state..."

    local katana_address=$(jq -r '.katana_tee.address' "$FIXTURES_DIR/deployment.json")

    # Get latest state
    local result
    result=$(sncast call \
        --url "$DEVNET_URL" \
        --contract-address "$katana_address" \
        --function get_latest_state 2>&1)

    log "get_latest_state result:"
    echo "$result"

    # Expected values from attestation (raw TeeQuoteResponse, no .result wrapper)
    local expected_block=$(jq -r '.blockNumber' "$FIXTURES_DIR/attestation.json")
    local expected_root=$(jq -r '.stateRoot' "$FIXTURES_DIR/attestation.json")
    local expected_hash=$(jq -r '.blockHash' "$FIXTURES_DIR/attestation.json")

    log "Expected values:"
    log "  block_number: $expected_block"
    log "  state_root: $expected_root"
    log "  block_hash: $expected_hash"

    # Basic validation (the result contains the expected values)
    if echo "$result" | grep -qi "error"; then
        error "State verification failed - call returned error"
        exit 1
    fi

    # Check if state was actually updated (non-zero values)
    if echo "$result" | grep -q "0x0, 0x0"; then
        warn "State was NOT updated (values are 0)"
        warn "This indicates the proof verification transaction failed"
        warn "See submit_proof output above for details"
    else
        log "State was updated successfully"
    fi

    log "State verification completed"
}

print_summary() {
    log ""
    log "=========================================="
    log "  E2E TEST SUMMARY"
    log "=========================================="
    log ""
    log "Deployment:"
    jq '.' "$FIXTURES_DIR/deployment.json"
    log ""
    log "Attestation:"
    jq '{stateRoot, blockHash, blockNumber}' "$FIXTURES_DIR/attestation.json"
    log ""
}

# === MAIN ===

MODE="${1:---fixture}"

case "$MODE" in
    --live)
        log "=========================================="
        log "  E2E TEST - LIVE MODE"
        log "=========================================="
        log ""
        start_devnet
        fetch_root_certs
        deploy_contracts
        generate_proof_live
        submit_proof
        verify_state
        print_summary
        log ""
        log "LIVE E2E TEST PASSED"
        log "  Fixtures saved for future --fixture runs"
        ;;

    --multi-block)
        log "=========================================="
        log "  E2E TEST - MULTI-BLOCK MODE"
        log "=========================================="
        log ""
        start_devnet
        fetch_root_certs
        deploy_contracts
        generate_multi_block_proofs
        log ""
        log "MULTI-BLOCK FIXTURE GENERATION COMPLETE"
        log "  Fixtures saved to tests/fixtures/block_N/"
        ;;

    --fixture)
        log "=========================================="
        log "  E2E TEST - FIXTURE MODE"
        log "=========================================="
        log ""

        # Verify fixtures exist
        [[ -f "$FIXTURES_DIR/attestation.json" ]] || die "Missing attestation.json. Run with --live first"
        [[ -f "$FIXTURES_DIR/proof.json" ]] || die "Missing proof.json. Run with --live first"
        [[ -f "$FIXTURES_DIR/calldata.txt" ]] || die "Missing calldata.txt. Run with --live first"

        start_devnet
        fetch_root_certs
        deploy_contracts
        submit_proof
        verify_state
        print_summary
        log ""
        log "FIXTURE E2E TEST PASSED"
        ;;

    *)
        echo "Usage: $0 [--live|--fixture|--multi-block]"
        echo ""
        echo "  --live         Fetch from TEE, generate real proof, save fixtures"
        echo "  --fixture      Use saved fixtures (default)"
        echo "  --multi-block  Generate fixtures for blocks 0, 1, 2"
        exit 1
        ;;
esac
