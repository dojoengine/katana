#!/bin/bash
# ==============================================================================
# VERIFY-BUILD.SH - Verify reproducibility of TEE builds
# ==============================================================================
#
# Computes and displays checksums for all TEE build artifacts.
# Run this after building to verify reproducibility.
#
# Usage:
#   ./verify-build.sh [OUTPUT_DIR]
#
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUTPUT_DIR="${1:-${SCRIPT_DIR}/output/qemu}"

if [[ ! -d "$OUTPUT_DIR" ]]; then
    echo "ERROR: Output directory not found: $OUTPUT_DIR"
    echo ""
    echo "Usage: $0 [OUTPUT_DIR]"
    echo "  Default: ${SCRIPT_DIR}/output/qemu"
    exit 1
fi

echo "=========================================="
echo "TEE Build Verification"
echo "=========================================="
echo "Output directory: $OUTPUT_DIR"
echo "Date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo ""

echo "Artifact Checksums (SHA256):"
echo "-------------------------------------------"

for file in OVMF.fd vmlinuz initrd.img katana; do
    if [[ -f "$OUTPUT_DIR/$file" ]]; then
        CHECKSUM=$(sha256sum "$OUTPUT_DIR/$file" | awk '{print $1}')
        SIZE=$(du -h "$OUTPUT_DIR/$file" | awk '{print $1}')
        printf "%-12s %s (%s)\n" "$file:" "$CHECKSUM" "$SIZE"
    else
        printf "%-12s <not found>\n" "$file:"
    fi
done

echo ""
echo "-------------------------------------------"

if [[ -f "$OUTPUT_DIR/build-info.txt" ]]; then
    echo ""
    echo "Build Configuration:"
    echo "-------------------------------------------"
    grep -E "^(SOURCE_DATE_EPOCH|OVMF_COMMIT|KERNEL_VERSION)=" "$OUTPUT_DIR/build-info.txt" 2>/dev/null || true
    echo "-------------------------------------------"
fi

echo ""
echo "To verify reproducibility:"
echo "  1. Save this output: $0 > build1.txt"
echo "  2. Clean and rebuild with same SOURCE_DATE_EPOCH"
echo "  3. Compare: diff build1.txt build2.txt"
echo ""
