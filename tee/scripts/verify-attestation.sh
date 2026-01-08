#!/usr/bin/env bash
#
# verify-attestation.sh - Extract and verify SEV-SNP launch measurement from attestation
#
# This script:
# 1. Calls the tee_generateQuote RPC endpoint
# 2. Extracts the attestation report from the response
# 3. Parses the launch measurement from the report
# 4. Compares it with the expected measurement
#
# Usage:
#   ./verify-attestation.sh [RPC_URL] [EXPECTED_MEASUREMENT_FILE]
#
# Requirements:
#   - curl
#   - jq
#   - xxd
#
# SEV-SNP Attestation Report Structure:
#   The report is 1184 bytes with the following key fields:
#   - Offset 0x00: Version (4 bytes)
#   - Offset 0x90: Measurement (48 bytes) <- This is the launch measurement!
#   - Offset 0x1C0: Report Data (64 bytes) - Contains Poseidon(state_root, block_hash)
#

set -euo pipefail

# Configuration
RPC_URL="${1:-http://localhost:5050}"
EXPECTED_MEASUREMENT_FILE="${2:-expected-measurement.txt}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

log_info() {
    echo -e "${BLUE}[INFO]${NC} $*"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $*"
}

log_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $*"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $*"
}

# Check dependencies
check_dependencies() {
    local missing_deps=()

    for cmd in curl jq xxd; do
        if ! command -v "$cmd" &> /dev/null; then
            missing_deps+=("$cmd")
        fi
    done

    if [ ${#missing_deps[@]} -gt 0 ]; then
        log_error "Missing required dependencies: ${missing_deps[*]}"
        log_info "Install with: sudo apt-get install curl jq xxd"
        exit 1
    fi
}

# Parse AMD SEV-SNP attestation report to extract launch measurement
extract_measurement_from_report() {
    local report_hex="$1"

    # Remove 0x prefix if present
    report_hex="${report_hex#0x}"

    # Convert hex string to binary
    local report_bin
    report_bin=$(echo "$report_hex" | xxd -r -p)

    # Extract measurement at offset 0x90 (144 bytes), length 48 bytes
    # The measurement field is at offset 0x90 in the attestation report
    local measurement
    measurement=$(echo "$report_bin" | dd bs=1 skip=144 count=48 2>/dev/null | xxd -p -c 48)

    echo "$measurement"
}

# Call the tee_generateQuote RPC endpoint
call_generate_quote() {
    local url="$1"

    log_info "Calling tee_generateQuote at $url"

    local response
    response=$(curl -s -X POST "$url" \
        -H "Content-Type: application/json" \
        -d '{
            "jsonrpc": "2.0",
            "method": "tee_generateQuote",
            "params": [],
            "id": 1
        }')

    # Check for errors
    if echo "$response" | jq -e '.error' > /dev/null 2>&1; then
        local error_msg
        error_msg=$(echo "$response" | jq -r '.error.message')
        log_error "RPC error: $error_msg"

        # Check if it's a "not supported" error (not running in TEE)
        if echo "$error_msg" | grep -qi "not supported\|not available"; then
            log_warning "This appears to be running on non-SEV-SNP hardware"
            log_info "This script must be run on actual AMD SEV-SNP capable hardware"
            log_info "with Katana started with --tee.provider sev-snp"
        fi

        exit 1
    fi

    echo "$response"
}

main() {
    echo "==========================================="
    echo "SEV-SNP Attestation Verification"
    echo "==========================================="
    echo ""

    check_dependencies

    # Check if expected measurement file exists
    if [ ! -f "$EXPECTED_MEASUREMENT_FILE" ]; then
        log_error "Expected measurement file not found: $EXPECTED_MEASUREMENT_FILE"
        log_info "Generate it with: ./calculate-measurement.sh ..."
        exit 1
    fi

    # Read expected measurement
    local expected_measurement
    expected_measurement=$(cat "$EXPECTED_MEASUREMENT_FILE")
    log_info "Expected measurement: $expected_measurement"
    echo ""

    # Call RPC endpoint
    log_info "Requesting attestation quote from Katana..."
    local response
    response=$(call_generate_quote "$RPC_URL")

    # Extract quote from response
    local quote_hex
    quote_hex=$(echo "$response" | jq -r '.result.quote')

    if [ "$quote_hex" = "null" ] || [ -z "$quote_hex" ]; then
        log_error "Failed to extract quote from response"
        echo "$response" | jq '.'
        exit 1
    fi

    # Extract blockchain state info
    local block_number
    local block_hash
    local state_root
    block_number=$(echo "$response" | jq -r '.result.blockNumber')
    block_hash=$(echo "$response" | jq -r '.result.blockHash')
    state_root=$(echo "$response" | jq -r '.result.stateRoot')

    log_success "Received attestation quote"
    log_info "  Block Number: $block_number"
    log_info "  Block Hash: $block_hash"
    log_info "  State Root: $state_root"
    log_info "  Quote Size: $((${#quote_hex} / 2)) bytes"
    echo ""

    # Extract measurement from attestation report
    log_info "Extracting launch measurement from attestation report..."
    local actual_measurement
    actual_measurement=$(extract_measurement_from_report "$quote_hex")

    if [ -z "$actual_measurement" ]; then
        log_error "Failed to extract measurement from attestation report"
        exit 1
    fi

    log_info "Actual measurement:   $actual_measurement"
    echo ""

    # Compare measurements
    log_info "Comparing measurements..."

    if [ "$actual_measurement" = "$expected_measurement" ]; then
        log_success "✓ Measurements match!"
        echo ""
        echo "==========================================="
        log_success "Attestation verification PASSED"
        echo "==========================================="
        echo ""
        log_info "The running Katana instance was launched with the expected"
        log_info "boot components (kernel + initrd + OVMF + cmdline)."
        log_info ""
        log_info "This proves:"
        log_info "  1. The Katana binary matches the reproducible build"
        log_info "  2. The kernel and initrd have not been tampered with"
        log_info "  3. The launch measurement is cryptographically bound to the build"
        echo ""
        exit 0
    else
        log_error "✗ Measurements do NOT match!"
        echo ""
        echo "Expected: $expected_measurement"
        echo "Actual:   $actual_measurement"
        echo ""
        log_warning "This indicates the running instance was NOT launched with"
        log_warning "the expected boot components. Possible causes:"
        log_warning "  1. Different kernel or initrd was used"
        log_warning "  2. The Katana binary was modified"
        log_warning "  3. Different OVMF firmware or kernel cmdline"
        log_warning "  4. The expected measurement file is outdated"
        echo ""
        echo "==========================================="
        log_error "Attestation verification FAILED"
        echo "==========================================="
        exit 1
    fi
}

# Show usage if --help
if [ "${1:-}" = "--help" ] || [ "${1:-}" = "-h" ]; then
    cat <<EOF
Usage: $0 [RPC_URL] [EXPECTED_MEASUREMENT_FILE]

Extract and verify SEV-SNP launch measurement from attestation.

Arguments:
  RPC_URL                  Katana RPC endpoint (default: http://localhost:5050)
  EXPECTED_MEASUREMENT_FILE  File containing expected measurement (default: expected-measurement.txt)

Example:
  # On SEV-SNP hardware with Katana running:
  $0 http://localhost:5050 expected-measurement.txt

Requirements:
  - Running on AMD SEV-SNP capable hardware
  - Katana started with --tee.provider sev-snp
  - Dependencies: curl, jq, xxd

SEV-SNP Attestation Report:
  The attestation report is 1184 bytes and contains the launch
  measurement at offset 0x90 (48 bytes). This measurement is a
  cryptographic hash of all boot components:
    - OVMF firmware
    - Linux kernel
    - Initrd (containing Katana binary)
    - Kernel command line

Exit Codes:
  0 - Measurements match (verification passed)
  1 - Measurements don't match or error occurred
EOF
    exit 0
fi

main "$@"
