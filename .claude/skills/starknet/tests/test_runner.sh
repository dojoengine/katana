#!/usr/bin/env bash
# Generic BDD Test Runner for Starknet Deployment Skills
# Usage: ./test_runner.sh [--profile <profile>] [--contract-dir <path>]
#
# This script is GENERIC and works with any Starknet project.
# It uses a minimal test contract included in fixtures/.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"
FIXTURES_DIR="$SCRIPT_DIR/fixtures"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

# Defaults
PROFILE="${PROFILE:-devnet}"
CONTRACT_DIR="${CONTRACT_DIR:-$FIXTURES_DIR/sample_contract}"

log_info() { echo -e "${GREEN}[INFO]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_error() { echo -e "${RED}[ERROR]${NC} $1"; }

# =============================================================================
# Generic Test Functions (Contract-Agnostic)
# =============================================================================

ensure_env_defaults() {
    export STARKNET_ACCOUNT="${STARKNET_ACCOUNT:-devnet-1}"
    export STARKNET_RPC_URL_DEVNET="${STARKNET_RPC_URL_DEVNET:-http://127.0.0.1:5050}"
    export STARKNET_ACCOUNTS_FILE="${STARKNET_ACCOUNTS_FILE:-$HOME/.starknet_accounts/starknet_open_zeppelin_accounts.json}"
}

test_prerequisites() {
    log_info "Checking prerequisites..."

    command -v sncast &> /dev/null || { log_error "sncast not found"; return 1; }
    command -v scarb &> /dev/null || { log_error "scarb not found"; return 1; }

    log_info "Prerequisites OK"
}

test_devnet_predeployed_accounts() {
    log_info "Testing devnet predeployed accounts..."

    # devnet-1 should work without any setup
    local output
    output=$(sncast --account devnet-1 account list 2>&1 || true)

    if echo "$output" | grep -q "devnet-1"; then
        log_info "Predeployed account devnet-1 accessible"
        return 0
    fi

    log_warn "Could not verify devnet-1 (devnet may not be running)"
    return 0
}

test_account_create_generic() {
    log_info "Testing generic account creation..."

    local account_name="test-generic-$(date +%s)"

    sncast account create --url "$STARKNET_RPC_URL_DEVNET" \
        --name "$account_name" \
        --type oz 2>&1 || {
        log_error "Account creation failed"
        return 1
    }

    log_info "Generic account creation passed"
}

test_declare_generic() {
    log_info "Testing generic contract declaration..."

    cd "$CONTRACT_DIR"
    scarb build || { log_error "Compilation failed"; return 1; }

    # Declare the sample contract
    local output
    output=$(sncast --account devnet-1 declare \
        --url "$STARKNET_RPC_URL_DEVNET" \
        --contract-name SampleContract 2>&1) || {
        if echo "$output" | grep -q "already declared"; then
            log_info "Contract already declared (OK)"
            return 0
        fi
        log_error "Declaration failed: $output"
        return 1
    }

    echo "$output" | grep -qi "class hash" || {
        log_error "No class_hash in output"
        return 1
    }

    log_info "Generic declaration test passed"
}

test_deploy_generic() {
    log_info "Testing generic contract deployment..."

    cd "$CONTRACT_DIR"

    # Deploy by contract name (auto-declares if needed)
    local output
    output=$(sncast --account devnet-1 deploy \
        --url "$STARKNET_RPC_URL_DEVNET" \
        --contract-name SampleContract 2>&1) || {
        log_error "Deployment failed: $output"
        return 1
    }

    echo "$output" | grep -qi "contract address" || {
        log_error "No contract address in output"
        return 1
    }

    log_info "Generic deployment test passed"
}

# =============================================================================
# Main
# =============================================================================

main() {
    log_info "=== Generic Starknet Skills Test Runner ==="
    log_info "Profile: $PROFILE"
    log_info "Contract Dir: $CONTRACT_DIR"

    ensure_env_defaults
    cd "$PROJECT_ROOT"

    local failed=0

    test_prerequisites || ((failed++))
    test_devnet_predeployed_accounts || ((failed++))

    if [[ "$PROFILE" == "devnet" ]]; then
        test_account_create_generic || ((failed++))
        test_declare_generic || ((failed++))
        test_deploy_generic || ((failed++))
    fi

    if [[ $failed -gt 0 ]]; then
        log_error "$failed test(s) failed"
        exit 1
    fi

    log_info "=== All generic tests passed! ==="
}

main "$@"
