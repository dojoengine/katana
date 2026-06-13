#!/bin/bash
# ==============================================================================
# BUILD-QEMU.SH - Build QEMU 10.2.0 from source
# ==============================================================================
#
# Builds QEMU with SEV-SNP support. Only tested with version 10.2.0.
#
# Usage:
#   ./build-qemu.sh           # Interactive prompt for install location
#   ./build-qemu.sh --global  # Install to /usr/local (requires sudo)
#   ./build-qemu.sh --local   # Install to misc/AMDSEV/output/qemu
#
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

QEMU_VERSION="10.2.0"
QEMU_URL="https://download.qemu.org/qemu-${QEMU_VERSION}.tar.xz"
QEMU_SHA256="f9e26a347be23a1b5fc5c10a502ea2571772064e281dd0cbb785f9eb96f47226"

function usage() {
    echo "Usage: $0 [OPTIONS]"
    echo ""
    echo "Build QEMU ${QEMU_VERSION} from source with SEV-SNP support."
    echo ""
    echo "OPTIONS:"
    echo "  --global      Install globally to /usr/local (requires sudo)"
    echo "  --local       Install to ${SCRIPT_DIR}/output/qemu (default)"
    echo "  --prefix DIR  Install to custom directory"
    echo "  -h, --help    Show this help"
    echo ""
    echo "If no option is provided, the script will prompt for installation location."
    exit 1
}

run_cmd() {
    echo "$*"
    eval "$*" || {
        echo "ERROR: $*"
        exit 1
    }
}

DEST=""
INSTALL_GLOBAL=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --global)
            INSTALL_GLOBAL=true
            DEST="/usr/local"
            shift
            ;;
        --local)
            DEST="${SCRIPT_DIR}/output/qemu"
            shift
            ;;
        --prefix)
            [[ -z "${2:-}" ]] && usage
            DEST="$2"
            shift 2
            ;;
        -h|--help)
            usage
            ;;
        *)
            echo "Unknown option: $1"
            usage
            ;;
    esac
done

# If no destination specified, prompt user
if [[ -z "$DEST" ]]; then
    echo "Where would you like to install QEMU ${QEMU_VERSION}?"
    echo ""
    echo "  1) Global (/usr/local) - requires sudo"
    echo "  2) Local (${SCRIPT_DIR}/output/qemu) - default"
    echo ""
    read -p "Select [1/2] (default: 2): " choice
    case "${choice:-2}" in
        1)
            INSTALL_GLOBAL=true
            DEST="/usr/local"
            ;;
        *)
            DEST="${SCRIPT_DIR}/output/qemu"
            ;;
    esac
fi

echo "=========================================="
echo "Building QEMU ${QEMU_VERSION}"
echo "=========================================="
echo "Configuration:"
echo "  Output dir:    $DEST"
echo "  Version:       $QEMU_VERSION"
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

# Download QEMU
echo "Downloading QEMU ${QEMU_VERSION}..."
run_cmd wget -q --show-progress "${QEMU_URL}"

# Verify checksum
echo "Verifying checksum..."
ACTUAL_SHA256=$(sha256sum "qemu-${QEMU_VERSION}.tar.xz" | awk '{print $1}')
if [[ "$ACTUAL_SHA256" != "$QEMU_SHA256" ]]; then
    echo "ERROR: Checksum mismatch!"
    echo "  Expected: $QEMU_SHA256"
    echo "  Actual:   $ACTUAL_SHA256"
    exit 1
fi
echo "[OK] Checksum verified"

# Extract
echo "Extracting..."
run_cmd tar xJf "qemu-${QEMU_VERSION}.tar.xz"

# Build
cd "qemu-${QEMU_VERSION}"
echo "Configuring..."
run_cmd ./configure --target-list=x86_64-softmmu --prefix="$DEST"

echo "Building (this may take a while)..."
run_cmd make -j"$(nproc)"

# Install
echo "Installing to $DEST..."
if [[ "$INSTALL_GLOBAL" == true ]]; then
    run_cmd sudo make install
else
    mkdir -p "$DEST"
    run_cmd make install
fi

popd >/dev/null

echo ""
echo "=========================================="
echo "[OK] QEMU build complete"
echo "=========================================="
echo "Installed to: $DEST"
echo "Binary: $DEST/bin/qemu-system-x86_64"
echo "Version: $("$DEST/bin/qemu-system-x86_64" --version | head -1)"
echo "=========================================="
