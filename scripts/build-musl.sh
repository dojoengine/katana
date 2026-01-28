#!/usr/bin/env bash
#
# Build a statically linked Katana binary using musl C runtime.
# Produces bit-for-bit identical builds when SOURCE_DATE_EPOCH is set.
#
# Usage:
#   ./scripts/build-musl.sh
#   SOURCE_DATE_EPOCH=$(git log -1 --format=%ct) ./scripts/build-musl.sh
#
# Prerequisites (Debian/Ubuntu):
#   sudo apt-get install musl-tools musl-dev clang libclang-dev gcc
#
# Prerequisites (Arch Linux):
#   sudo pacman -S musl clang gcc
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

cd "$PROJECT_ROOT"

# Check for required tools and install if missing
MISSING_PKGS=()

if ! command -v cargo &> /dev/null; then
    echo "ERROR: cargo is not installed. Install Rust via rustup: https://rustup.rs"
    exit 1
fi

if ! command -v rustup &> /dev/null; then
    echo "ERROR: rustup is not installed. Install via: https://rustup.rs"
    exit 1
fi

if ! command -v musl-gcc &> /dev/null; then
    MISSING_PKGS+=(musl-tools)
fi

if ! command -v clang &> /dev/null; then
    MISSING_PKGS+=(clang)
fi

# Install missing packages if any
if [[ ${#MISSING_PKGS[@]} -gt 0 ]]; then
    echo "Installing missing packages: ${MISSING_PKGS[*]}"
    if command -v apt-get &> /dev/null; then
        sudo apt-get update && sudo apt-get install -y "${MISSING_PKGS[@]}"
    elif command -v pacman &> /dev/null; then
        # Map package names for Arch Linux
        ARCH_PKGS=()
        for pkg in "${MISSING_PKGS[@]}"; do
            case "$pkg" in
                musl-tools) ARCH_PKGS+=(musl) ;;
                *) ARCH_PKGS+=("$pkg") ;;
            esac
        done
        sudo pacman -S --noconfirm "${ARCH_PKGS[@]}"
    else
        echo "ERROR: Cannot auto-install packages. Please install manually: ${MISSING_PKGS[*]}"
        exit 1
    fi
fi

# Add musl target if not already installed
if ! rustup target list --installed | grep -q x86_64-unknown-linux-musl; then
    echo "Adding x86_64-unknown-linux-musl target..."
    rustup target add x86_64-unknown-linux-musl
fi

# Set SOURCE_DATE_EPOCH for reproducible builds if not already set
if [[ -z "${SOURCE_DATE_EPOCH:-}" ]]; then
    SOURCE_DATE_EPOCH=$(git log -1 --format=%ct 2>/dev/null || date +%s)
    echo "SOURCE_DATE_EPOCH not set, using: $SOURCE_DATE_EPOCH"
fi
export SOURCE_DATE_EPOCH

# Use musl-gcc wrapper for proper static linking
export CC_x86_64_unknown_linux_musl=musl-gcc
export CFLAGS_x86_64_unknown_linux_musl="-lgcc"

# Reproducibility environment variables
# -C link-arg=-lgcc: link libgcc for CPU intrinsics used by reth-mdbx-sys
# -C link-arg=-s: strip symbols for bit-for-bit identity
export RUSTFLAGS="--remap-path-prefix=$PROJECT_ROOT=/build --remap-path-prefix=$HOME/.cargo=/cargo -C target-feature=+crt-static -C link-arg=-lgcc -C link-arg=-s"
export LANG=C.UTF-8
export LC_ALL=C.UTF-8
export TZ=UTC

echo "Building Katana with musl (static linking)..."
echo "  SOURCE_DATE_EPOCH: $SOURCE_DATE_EPOCH"
echo "  RUSTFLAGS: $RUSTFLAGS"

# Build the binary
cargo build \
    --locked \
    --target x86_64-unknown-linux-musl \
    --profile performance \
    --no-default-features \
    --features "cartridge,client,init-slot,jemalloc" \
    --bin katana

BINARY_PATH="$PROJECT_ROOT/target/x86_64-unknown-linux-musl/performance/katana"

if [[ -f "$BINARY_PATH" ]]; then
    echo ""
    echo "Build successful!"
    echo "Binary: $BINARY_PATH"
    echo ""
    file "$BINARY_PATH"
    ls -lh "$BINARY_PATH"
else
    echo "ERROR: Binary not found at $BINARY_PATH"
    exit 1
fi
