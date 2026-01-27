#!/bin/bash
# ==============================================================================
# BUILD-KERNEL.SH - Download and extract Ubuntu kernel for TEE
# ==============================================================================
#
# Downloads pinned Ubuntu kernel package and extracts vmlinuz.
#
# Usage:
#   ./build-kernel.sh OUTPUT_DIR
#
# Environment (required):
#   KERNEL_VERSION  Kernel version to download (e.g., 6.8.0-90)
#
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

function usage() {
    echo "Usage: $0 OUTPUT_DIR"
    echo ""
    echo "Download and extract Ubuntu kernel for TEE."
    echo ""
    echo "ARGUMENTS:"
    echo "  OUTPUT_DIR    Directory to store vmlinuz"
    echo ""
    echo "ENVIRONMENT VARIABLES (or source build-config):"
    echo "  KERNEL_VERSION  Kernel version to download (e.g., 6.8.0-90)"
    echo ""
    echo "EXAMPLES:"
    echo "  source build-config && $0 ./output"
    echo "  KERNEL_VERSION=6.8.0-90 $0 ./output"
    exit 1
}

run_cmd() {
    echo "$*"
    eval "$*" || {
        echo "ERROR: $*"
        exit 1
    }
}

if [[ $# -lt 1 ]] || [[ "${1:-}" == "-h" ]] || [[ "${1:-}" == "--help" ]]; then
    usage
fi

DEST="$1"

# Validate required environment variables
KERNEL_VER="${KERNEL_VERSION:?KERNEL_VERSION not set - source build-config first}"

echo "=========================================="
echo "Building Kernel"
echo "=========================================="
echo "Configuration:"
echo "  Output dir:      $DEST"
echo "  Kernel version:  $KERNEL_VER"
echo "=========================================="
echo ""

# Create temporary working directory
WORK_DIR=$(mktemp -d)

cleanup() {
    local exit_code=$?
    if [[ -d "$WORK_DIR" ]]; then
        rm -rf "$WORK_DIR"
    fi
    exit $exit_code
}

trap cleanup EXIT INT TERM

echo "Working directory: $WORK_DIR"

pushd "$WORK_DIR" >/dev/null
    # Download kernel package
    run_cmd apt-get download linux-image-unsigned-${KERNEL_VER}-generic

    echo ""
    echo "Downloaded packages:"
    ls -lh *.deb

    # Verify package checksum if provided
    if [[ -n "${KERNEL_PKG_SHA256:-}" ]]; then
        echo ""
        echo "Verifying package checksum..."
        ACTUAL_SHA256=$(sha256sum linux-image-unsigned-*.deb | awk '{print $1}')
        if [[ "$ACTUAL_SHA256" != "$KERNEL_PKG_SHA256" ]]; then
            echo "ERROR: Package checksum mismatch!"
            echo "  Expected: $KERNEL_PKG_SHA256"
            echo "  Actual:   $ACTUAL_SHA256"
            exit 1
        fi
        echo "[OK] Package checksum verified: $ACTUAL_SHA256"
    fi

    # Extract kernel
    mkdir -p extracted
    run_cmd dpkg-deb -x linux-image-unsigned-*.deb extracted/

    # Copy to output
    mkdir -p "$DEST"
    run_cmd cp extracted/boot/vmlinuz-* "$DEST/vmlinuz"
popd >/dev/null

echo ""
echo "=========================================="
echo "[OK] Kernel build complete"
echo "=========================================="
echo "Output: $DEST/vmlinuz"
echo "Version: ${KERNEL_VER}"
echo "SHA256: $(sha256sum "$DEST/vmlinuz" | awk '{print $1}')"
echo "=========================================="
