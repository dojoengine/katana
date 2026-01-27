#!/bin/bash
# Build TEE components for Katana (AMD SEV-SNP)
#
# This script orchestrates the two-stage Docker build process:
# 1. Build katana binary using reproducible.Dockerfile
# 2. Build VM image (kernel, initrd, OVMF) using vm-image.Dockerfile
#
# Usage:
#   ./tee/scripts/build-tee-components.sh [--output-dir DIR]
#
# Output:
#   vmlinuz        - Linux kernel
#   initrd.img     - initramfs with embedded katana
#   ovmf.fd        - AMD SEV-SNP OVMF firmware
#   katana-binary  - Standalone katana binary
#   build-info.txt - Build metadata

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
OUTPUT_DIR="${REPO_ROOT}/tee/output"

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --output-dir)
            OUTPUT_DIR="$2"
            shift 2
            ;;
        -h|--help)
            echo "Usage: $0 [--output-dir DIR]"
            echo ""
            echo "Build TEE components for Katana (AMD SEV-SNP)"
            echo ""
            echo "Options:"
            echo "  --output-dir DIR  Output directory (default: tee/output)"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

cd "$REPO_ROOT"

# Use git commit timestamp for reproducibility
SOURCE_DATE_EPOCH=$(git log -1 --format=%ct)
echo "SOURCE_DATE_EPOCH: $SOURCE_DATE_EPOCH ($(date -d @$SOURCE_DATE_EPOCH -u +%Y-%m-%dT%H:%M:%SZ))"

# Create output directory
mkdir -p "$OUTPUT_DIR"

echo ""
echo "=== Stage 1: Building Katana binary (reproducible) ==="
echo ""

# TEMP: Use local musl build instead of Docker
export SOURCE_DATE_EPOCH
"$SCRIPT_DIR/../../scripts/build-musl.sh"
cp "$REPO_ROOT/target/x86_64-unknown-linux-musl/performance/katana" "$OUTPUT_DIR/katana-binary"

# # Docker-based reproducible build (commented out for local musl build)
# docker build \
#     -f reproducible.Dockerfile \
#     --build-arg SOURCE_DATE_EPOCH="$SOURCE_DATE_EPOCH" \
#     -t katana-reproducible \
#     .
#
# # Extract the binary
# KATANA_CONTAINER=$(docker create katana-reproducible)
# docker cp "$KATANA_CONTAINER:/katana" "$OUTPUT_DIR/katana-binary"
# docker rm "$KATANA_CONTAINER"

echo "Katana binary built: $OUTPUT_DIR/katana-binary"
echo "SHA256: $(sha256sum "$OUTPUT_DIR/katana-binary" | cut -d' ' -f1)"

echo ""
echo "=== Stage 2: Building VM image components ==="
echo ""

# Copy katana binary to expected location for vm-image.Dockerfile
cp "$OUTPUT_DIR/katana-binary" "$REPO_ROOT/katana-binary"

docker build \
    -f vm-image.Dockerfile \
    --build-arg SOURCE_DATE_EPOCH="$SOURCE_DATE_EPOCH" \
    --target initrd-builder \
    -t katana-vm-image \
    .

# Clean up temporary binary
rm -f "$REPO_ROOT/katana-binary"

# Extract from initrd-builder stage
VM_CONTAINER=$(docker create katana-vm-image)
docker cp "$VM_CONTAINER:/components/vmlinuz" "$OUTPUT_DIR/vmlinuz"
docker cp "$VM_CONTAINER:/components/initrd.img" "$OUTPUT_DIR/initrd.img"
docker cp "$VM_CONTAINER:/components/ovmf.fd" "$OUTPUT_DIR/ovmf.fd"
docker cp "$VM_CONTAINER:/components/build-info.txt" "$OUTPUT_DIR/build-info.txt"
# Copy OVMF CODE and VARS if they exist (for split pflash usage)
docker cp "$VM_CONTAINER:/ovmf-output/ovmf_code.fd" "$OUTPUT_DIR/ovmf_code.fd" 2>/dev/null || true
docker cp "$VM_CONTAINER:/ovmf-output/ovmf_vars.fd" "$OUTPUT_DIR/ovmf_vars.fd" 2>/dev/null || true
docker rm "$VM_CONTAINER"

# Add katana info to build-info.txt
echo "KATANA_SHA256=$(sha256sum "$OUTPUT_DIR/katana-binary" | cut -d' ' -f1)" >> "$OUTPUT_DIR/build-info.txt"
echo "SOURCE_DATE_EPOCH=$SOURCE_DATE_EPOCH" >> "$OUTPUT_DIR/build-info.txt"

echo ""
echo "=== Build complete ==="
echo ""
echo "Output directory: $OUTPUT_DIR"
echo ""
ls -lh "$OUTPUT_DIR"
echo ""
echo "Checksums:"
sha256sum "$OUTPUT_DIR"/{vmlinuz,initrd.img,ovmf.fd,katana-binary}
