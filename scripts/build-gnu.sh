#!/usr/bin/env bash
#
# Build a dynamically linked Katana binary using the GNU/glibc Linux target.
# Produces deterministic output when SOURCE_DATE_EPOCH is set and dependency
# inputs are unchanged.
#
# Usage:
#   ./scripts/build-gnu.sh
#   SOURCE_DATE_EPOCH=$(git log -1 --format=%ct) ./scripts/build-gnu.sh
#
# Prerequisites (Debian/Ubuntu):
#   sudo apt-get install clang gcc
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

STRICT_MODE=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --strict)
            STRICT_MODE=1
            shift
            ;;
        -h|--help)
            echo "Usage: $0 [--strict]"
            echo ""
            echo "Build a dynamically linked Katana binary using glibc."
            echo ""
            echo "OPTIONS:"
            echo "  --strict  Require vendored dependencies for reproducible builds"
            echo "  -h|--help Show this help message"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

cd "$PROJECT_ROOT"

if [[ "$(uname -s)" != "Linux" || "$(uname -m)" != "x86_64" ]]; then
    echo "ERROR: glibc Katana builds require x86_64 Linux."
    echo "       On other hosts, pass a pre-built Linux glibc binary with --katana."
    exit 1
fi

MISSING_PKGS=()

if ! command -v cargo &> /dev/null; then
    echo "ERROR: cargo is not installed. Install Rust via rustup: https://rustup.rs"
    exit 1
fi

if ! command -v rustup &> /dev/null; then
    echo "ERROR: rustup is not installed. Install via: https://rustup.rs"
    exit 1
fi

if ! command -v gcc &> /dev/null; then
    MISSING_PKGS+=(gcc)
fi

if ! command -v clang &> /dev/null; then
    MISSING_PKGS+=(clang)
fi

if [[ ${#MISSING_PKGS[@]} -gt 0 ]]; then
    echo "Installing missing packages: ${MISSING_PKGS[*]}"
    if command -v apt-get &> /dev/null; then
        sudo apt-get update && sudo apt-get install -y "${MISSING_PKGS[@]}"
    elif command -v pacman &> /dev/null; then
        sudo pacman -S --noconfirm "${MISSING_PKGS[@]}"
    else
        echo "ERROR: Cannot auto-install packages. Please install manually: ${MISSING_PKGS[*]}"
        exit 1
    fi
fi

if ! rustup target list --installed | grep -q x86_64-unknown-linux-gnu; then
    echo "Adding x86_64-unknown-linux-gnu target..."
    rustup target add x86_64-unknown-linux-gnu
fi

if [[ -z "${SOURCE_DATE_EPOCH:-}" ]]; then
    SOURCE_DATE_EPOCH=$(git log -1 --format=%ct 2>/dev/null || date +%s)
    echo "SOURCE_DATE_EPOCH not set, using: $SOURCE_DATE_EPOCH"
fi
export SOURCE_DATE_EPOCH

CARGO_HOME_DIR="${CARGO_HOME:-$HOME/.cargo}"

export RUSTFLAGS="--remap-path-prefix=$PROJECT_ROOT=/build --remap-path-prefix=$CARGO_HOME_DIR=/cargo -C link-arg=-Wl,--build-id=none -C link-arg=-s"
export LANG=C.UTF-8
export LC_ALL=C.UTF-8
export TZ=UTC

OFFLINE_FLAG=""
if [[ -d "$PROJECT_ROOT/vendor" ]] && [[ -f "$PROJECT_ROOT/.cargo/config.toml" ]]; then
    if grep -q '\[source.vendored-sources\]' "$PROJECT_ROOT/.cargo/config.toml" 2>/dev/null; then
        echo "Using vendored dependencies (reproducible mode)"
        OFFLINE_FLAG="--offline"
    fi
fi

if [[ -z "$OFFLINE_FLAG" ]]; then
    if [[ $STRICT_MODE -eq 1 ]]; then
        echo "ERROR: --strict mode requires vendored dependencies"
        echo "       Run: cargo vendor vendor/"
        echo "       Then add vendor config to .cargo/config.toml"
        exit 1
    else
        echo "WARNING: Vendored dependencies not found - build may not be reproducible"
        echo "         For reproducible builds, run: cargo vendor vendor/"
    fi
fi

echo ""
echo "Building Katana with glibc (dynamic linking)..."
echo "  SOURCE_DATE_EPOCH: $SOURCE_DATE_EPOCH"
echo "  RUSTFLAGS: $RUSTFLAGS"
echo "  OFFLINE_FLAG: ${OFFLINE_FLAG:-<none>}"

cargo build \
    $OFFLINE_FLAG \
    --locked \
    --target x86_64-unknown-linux-gnu \
    --profile performance \
    --no-default-features \
    --features "client,init-slot,jemalloc" \
    --bin katana

BINARY_PATH="$PROJECT_ROOT/target/x86_64-unknown-linux-gnu/performance/katana"

if [[ ! -f "$BINARY_PATH" ]]; then
    echo "ERROR: Binary not found at $BINARY_PATH"
    exit 1
fi

echo ""
echo "Build successful!"
echo "Binary: $BINARY_PATH"
echo ""
if command -v file &> /dev/null; then
    file "$BINARY_PATH"
fi
if command -v readelf &> /dev/null; then
    readelf -l "$BINARY_PATH" | grep 'Requesting program interpreter' || true
fi
ls -lh "$BINARY_PATH"
