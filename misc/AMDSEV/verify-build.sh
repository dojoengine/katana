#!/bin/bash
# ==============================================================================
# VERIFY-BUILD.SH - Verify reproducibility of TEE builds
# ==============================================================================
#
# Computes and displays checksums for TEE build artifacts.
#
# Usage:
#   ./verify-build.sh [OUTPUT_DIR]
#   ./verify-build.sh --compare OUTPUT_DIR_A OUTPUT_DIR_B
#
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ARTIFACTS=(OVMF.fd vmlinuz initrd.img katana build-info.txt materials.lock)

usage() {
    echo "Usage: $0 [OUTPUT_DIR]"
    echo "       $0 --compare OUTPUT_DIR_A OUTPUT_DIR_B"
    echo ""
    echo "Default OUTPUT_DIR: ${SCRIPT_DIR}/output/qemu"
    exit 1
}

checksum_file() {
    local file="$1"
    if [[ -f "$file" ]]; then
        sha256sum "$file" | awk '{print $1}'
    else
        echo "<missing>"
    fi
}

print_report() {
    local output_dir="$1"

    [[ -d "$output_dir" ]] || {
        echo "ERROR: Output directory not found: $output_dir" >&2
        exit 1
    }

    echo "=========================================="
    echo "TEE Build Verification"
    echo "=========================================="
    echo "Output directory: $output_dir"
    echo ""

    echo "Artifact Checksums (SHA256):"
    echo "-------------------------------------------"
    for file in "${ARTIFACTS[@]}"; do
        local path="$output_dir/$file"
        if [[ -f "$path" ]]; then
            local checksum
            local size
            checksum="$(sha256sum "$path" | awk '{print $1}')"
            size="$(du -h "$path" | awk '{print $1}')"
            printf "%-16s %s (%s)\n" "$file:" "$checksum" "$size"
        else
            printf "%-16s <not found>\n" "$file:"
        fi
    done

    if [[ -f "$output_dir/build-info.txt" ]]; then
        echo ""
        echo "Build Configuration:"
        echo "-------------------------------------------"
        grep -E "^(SOURCE_DATE_EPOCH|INPUT_MANIFEST_SHA256|OVMF_COMMIT|KERNEL_VERSION)=" "$output_dir/build-info.txt" || true
        echo "-------------------------------------------"
    fi

    echo ""
}

compare_reports() {
    local dir_a="$1"
    local dir_b="$2"
    local failed=0

    [[ -d "$dir_a" ]] || { echo "ERROR: Output directory not found: $dir_a" >&2; exit 1; }
    [[ -d "$dir_b" ]] || { echo "ERROR: Output directory not found: $dir_b" >&2; exit 1; }

    echo "=========================================="
    echo "TEE Build Reproducibility Compare"
    echo "=========================================="
    echo "A: $dir_a"
    echo "B: $dir_b"
    echo ""

    for file in "${ARTIFACTS[@]}"; do
        local checksum_a checksum_b
        checksum_a="$(checksum_file "$dir_a/$file")"
        checksum_b="$(checksum_file "$dir_b/$file")"

        if [[ "$checksum_a" == "$checksum_b" ]]; then
            printf "[OK] %-16s %s\n" "$file" "$checksum_a"
        else
            printf "[FAIL] %-16s A=%s B=%s\n" "$file" "$checksum_a" "$checksum_b"
            failed=1
        fi
    done

    if [[ $failed -ne 0 ]]; then
        echo ""
        echo "Reproducibility check failed."
        exit 1
    fi

    echo ""
    echo "Reproducibility check passed."
}

if [[ $# -eq 0 ]]; then
    print_report "${SCRIPT_DIR}/output/qemu"
    exit 0
fi

if [[ "$1" == "--compare" ]]; then
    [[ $# -eq 3 ]] || usage
    compare_reports "$2" "$3"
    exit 0
fi

[[ $# -eq 1 ]] || usage
print_report "$1"
