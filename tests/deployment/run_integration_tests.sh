#!/usr/bin/env bash
# Integration Test Runner for katana-tee contracts
# Usage: ./run_integration_tests.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
DEPLOYMENTS_DIR="$PROJECT_ROOT/deployments"

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

log_info() { echo -e "${GREEN}[INFO]${NC} $1"; }
log_error() { echo -e "${RED}[ERROR]${NC} $1"; }

# =============================================================================
# Integration Tests with amd_tee_registry + katana_tee
# =============================================================================

ensure_env_defaults() {
    export STARKNET_ACCOUNT="${STARKNET_ACCOUNT:-devnet-1}"
    export STARKNET_RPC_URL_DEVNET="${STARKNET_RPC_URL_DEVNET:-http://127.0.0.1:5050}"
    export STARKNET_ACCOUNTS_FILE="${STARKNET_ACCOUNTS_FILE:-$HOME/.starknet_accounts/starknet_open_zeppelin_accounts.json}"
}

test_compile_contracts() {
    log_info "Compiling contracts..."

    cd "$PROJECT_ROOT/contracts/amd_tee_registry"
    scarb build || return 1

    cd "$PROJECT_ROOT/contracts/katana_tee"
    scarb build || return 1

    log_info "Contracts compiled"
}

test_declare_amd_registry() {
    log_info "Declaring AMDTEERegistry..."

    cd "$PROJECT_ROOT/contracts/amd_tee_registry"

    # Compute class hash from artifacts (works even if already declared)
    AMD_CLASS_HASH=$(sncast utils class-hash \
        --contract-name AMDTEERegistry \
        --package amd_tee_registry 2>&1 | grep -oP '0x[a-fA-F0-9]+' | head -1)

    if [[ -z "$AMD_CLASS_HASH" ]]; then
        log_error "Unable to compute AMDTEERegistry class hash"
        return 1
    fi

    local output
    output=$(sncast --account devnet-1 declare \
        --url "$STARKNET_RPC_URL_DEVNET" \
        --contract-name AMDTEERegistry \
        --package amd_tee_registry 2>&1) || {
        if echo "$output" | grep -qi "already declared"; then
            log_info "AMDTEERegistry already declared (OK)"
        else
            log_error "Declaration failed: $output"
            return 1
        fi
    }

    log_info "AMDTEERegistry class_hash: $AMD_CLASS_HASH"
    export AMD_CLASS_HASH
}

test_deploy_amd_registry() {
    log_info "Deploying AMDTEERegistry with empty arrays..."

    cd "$PROJECT_ROOT/contracts/amd_tee_registry"

    local output
    output=$(sncast --account devnet-1 deploy \
        --url "$STARKNET_RPC_URL_DEVNET" \
        --class-hash "$AMD_CLASS_HASH" \
        --constructor-calldata 0 0 0 0 0 0 0 2>&1) || {
        log_error "Deployment failed: $output"
        return 1
    }

    AMD_ADDRESS=$(echo "$output" | grep -oP 'Contract Address:\s*\K0x[a-fA-F0-9]+')

    if [[ -z "$AMD_ADDRESS" ]]; then
        log_error "Unable to parse AMDTEERegistry contract address"
        return 1
    fi

    log_info "AMDTEERegistry deployed at: $AMD_ADDRESS"
    export AMD_ADDRESS
}

test_declare_katana_tee() {
    log_info "Declaring KatanaTee..."

    cd "$PROJECT_ROOT/contracts/katana_tee"

    KATANA_CLASS_HASH=$(sncast utils class-hash \
        --contract-name KatanaTee \
        --package katana_tee 2>&1 | grep -oP '0x[a-fA-F0-9]+' | head -1)

    if [[ -z "$KATANA_CLASS_HASH" ]]; then
        log_error "Unable to compute KatanaTee class hash"
        return 1
    fi

    local output
    output=$(sncast --account devnet-1 declare \
        --url "$STARKNET_RPC_URL_DEVNET" \
        --contract-name KatanaTee \
        --package katana_tee 2>&1) || {
        if echo "$output" | grep -qi "already declared"; then
            log_info "KatanaTee already declared (OK)"
        else
            log_error "Declaration failed: $output"
            return 1
        fi
    }

    log_info "KatanaTee class_hash: $KATANA_CLASS_HASH"
    export KATANA_CLASS_HASH
}

test_deploy_katana_tee() {
    log_info "Deploying KatanaTee with registry address..."

    cd "$PROJECT_ROOT/contracts/katana_tee"

    local output
    output=$(sncast --account devnet-1 deploy \
        --url "$STARKNET_RPC_URL_DEVNET" \
        --class-hash "$KATANA_CLASS_HASH" \
        --constructor-calldata "$AMD_ADDRESS" 2>&1) || {
        log_error "Deployment failed: $output"
        return 1
    }

    KATANA_ADDRESS=$(echo "$output" | grep -oP 'Contract Address:\s*\K0x[a-fA-F0-9]+')

    if [[ -z "$KATANA_ADDRESS" ]]; then
        log_error "Unable to parse KatanaTee contract address"
        return 1
    fi

    log_info "KatanaTee deployed at: $KATANA_ADDRESS"
    export KATANA_ADDRESS
}

test_verify_registry_address() {
    log_info "Verifying katana_tee.get_registry_address()..."

    local output
    output=$(sncast call \
        --url "$STARKNET_RPC_URL_DEVNET" \
        --contract-address "$KATANA_ADDRESS" \
        --function get_registry_address 2>&1) || {
        log_error "Call failed: $output"
        return 1
    }

    # Normalize: lowercase, strip 0x and leading zeros
    local expected
    local actual

    expected=$(echo "$AMD_ADDRESS" | tr 'A-F' 'a-f' | sed 's/^0x//' | sed 's/^0*//')
    actual=$(echo "$output" | grep -oP '0x[a-fA-F0-9]+' | head -1 | tr 'A-F' 'a-f' | sed 's/^0x//' | sed 's/^0*//')

    if [[ -n "$actual" && "$actual" == "$expected" ]]; then
        log_info "Registry address verified correctly"
    else
        log_error "Registry address mismatch! expected=0x${expected} actual=0x${actual}"
        return 1
    fi
}

save_deployments() {
    log_info "Saving deployments to $DEPLOYMENTS_DIR/devnet.json..."

    mkdir -p "$DEPLOYMENTS_DIR"

    cat > "$DEPLOYMENTS_DIR/devnet.json" << EOF_DEPLOYMENTS
{
  "network": "devnet",
  "timestamp": "$(date -Iseconds)",
  "contracts": {
    "amd_tee_registry": {
      "class_hash": "${AMD_CLASS_HASH}",
      "address": "${AMD_ADDRESS}"
    },
    "katana_tee": {
      "class_hash": "${KATANA_CLASS_HASH}",
      "address": "${KATANA_ADDRESS}"
    }
  }
}
EOF_DEPLOYMENTS

    log_info "Deployments saved"
}

# =============================================================================
# Main
# =============================================================================

main() {
    log_info "=== Integration Tests: amd_tee_registry + katana_tee ==="

    ensure_env_defaults

    local failed=0

    test_compile_contracts || ((failed++))
    test_declare_amd_registry || ((failed++))
    test_deploy_amd_registry || ((failed++))
    test_declare_katana_tee || ((failed++))
    test_deploy_katana_tee || ((failed++))
    test_verify_registry_address || ((failed++))
    save_deployments || ((failed++))

    if [[ $failed -gt 0 ]]; then
        log_error "$failed test(s) failed"
        exit 1
    fi

    log_info "=== All integration tests passed! ==="
}

main "$@"
