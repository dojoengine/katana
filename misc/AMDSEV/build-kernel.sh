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
#   SOURCE_DATE_EPOCH Unix timestamp for reproducible output metadata
#
# Environment (optional for stronger reproducibility):
#   APT_SNAPSHOT_URL         Snapshot apt base URL
#   APT_SNAPSHOT_SUITE       Snapshot suite (e.g., noble)
#   APT_SNAPSHOT_COMPONENTS  Snapshot components (e.g., "main")
#
# ==============================================================================

set -euo pipefail

# Environment normalization for reproducibility
export TZ=UTC
export LANG=C.UTF-8
export LC_ALL=C.UTF-8
umask 022

APT_SNAPSHOT_DIR=""
declare -a APT_OPTS=()
APT_SOURCE_MODE="host"

usage() {
    echo "Usage: $0 OUTPUT_DIR"
    echo ""
    echo "Download and extract Ubuntu kernel for TEE."
    echo ""
    echo "ARGUMENTS:"
    echo "  OUTPUT_DIR    Directory to store vmlinuz"
    echo ""
    echo "ENVIRONMENT VARIABLES (or source build-config):"
    echo "  KERNEL_VERSION          Kernel version to download (e.g., 6.8.0-90)"
    echo "  SOURCE_DATE_EPOCH       Unix timestamp for reproducible output metadata"
    echo "  APT_SNAPSHOT_URL        Optional snapshot apt URL for deterministic package resolution"
    echo "  APT_SNAPSHOT_SUITE      Snapshot suite (default from build-config)"
    echo "  APT_SNAPSHOT_COMPONENTS Snapshot components (default from build-config)"
    echo ""
    echo "EXAMPLES:"
    echo "  source build-config && $0 ./output"
    echo "  KERNEL_VERSION=6.8.0-90 $0 ./output"
    exit 1
}

die() {
    echo "ERROR: $*" >&2
    exit 1
}

run_cmd() {
    echo "$*"
    "$@" || die "$*"
}

setup_apt_snapshot() {
    : "${APT_SNAPSHOT_URL:?APT_SNAPSHOT_URL is required when snapshot mode is enabled}"
    : "${APT_SNAPSHOT_SUITE:?APT_SNAPSHOT_SUITE is required when snapshot mode is enabled}"

    local components="${APT_SNAPSHOT_COMPONENTS:-main}"
    local sources_list

    APT_SNAPSHOT_DIR="$(mktemp -d)"
    mkdir -p "$APT_SNAPSHOT_DIR/lists/partial" "$APT_SNAPSHOT_DIR/cache/partial"

    sources_list="$APT_SNAPSHOT_DIR/sources.list"
    printf "deb [check-valid-until=no] %s %s %s\n" \
        "$APT_SNAPSHOT_URL" "$APT_SNAPSHOT_SUITE" "$components" > "$sources_list"

    APT_OPTS=(
        -o "Dir::Etc::sourcelist=$sources_list"
        -o "Dir::Etc::sourceparts=-"
        -o "Dir::State::Lists=$APT_SNAPSHOT_DIR/lists"
        -o "Dir::Cache::archives=$APT_SNAPSHOT_DIR/cache"
        -o "Dir::State::status=/var/lib/dpkg/status"
        -o "APT::Get::List-Cleanup=0"
    )

    echo "Using apt snapshot source:"
    echo "  URL:        $APT_SNAPSHOT_URL"
    echo "  Suite:      $APT_SNAPSHOT_SUITE"
    echo "  Components: $components"
    run_cmd apt-get "${APT_OPTS[@]}" update
    APT_SOURCE_MODE="snapshot"
}

apt_download() {
    local package="$1"
    run_cmd apt-get "${APT_OPTS[@]}" download "$package"
}

cleanup() {
    local exit_code=$?
    if [[ -n "${WORK_DIR:-}" ]] && [[ -d "$WORK_DIR" ]]; then
        rm -rf "$WORK_DIR"
    fi
    if [[ -n "$APT_SNAPSHOT_DIR" ]] && [[ -d "$APT_SNAPSHOT_DIR" ]]; then
        rm -rf "$APT_SNAPSHOT_DIR"
    fi
    exit "$exit_code"
}

if [[ $# -lt 1 ]] || [[ "${1:-}" == "-h" ]] || [[ "${1:-}" == "--help" ]]; then
    usage
fi

DEST="$1"
KERNEL_VER="${KERNEL_VERSION:?KERNEL_VERSION not set - source build-config first}"
: "${KERNEL_PKG_SHA256:?KERNEL_PKG_SHA256 not set - required for reproducible builds}"
: "${SOURCE_DATE_EPOCH:?SOURCE_DATE_EPOCH not set - required for reproducible builds}"
if ! [[ "$SOURCE_DATE_EPOCH" =~ ^[0-9]+$ ]]; then
    die "SOURCE_DATE_EPOCH must be a unix timestamp integer"
fi

echo "=========================================="
echo "Building Kernel"
echo "=========================================="
echo "Configuration:"
echo "  Output dir:      $DEST"
echo "  Kernel version:  $KERNEL_VER"
echo "  SOURCE_DATE_EPOCH: $SOURCE_DATE_EPOCH"
echo "  APT source:      ${APT_SNAPSHOT_URL:-<host-configured sources>}"
echo "=========================================="
echo ""

if [[ -n "${APT_SNAPSHOT_URL:-}" ]]; then
    setup_apt_snapshot
else
    echo "WARNING: APT_SNAPSHOT_URL not set, using host apt sources (weaker reproducibility)"
fi

WORK_DIR="$(mktemp -d)"
trap cleanup EXIT INT TERM

echo "Working directory: $WORK_DIR"

pushd "$WORK_DIR" >/dev/null
    apt_download "linux-image-unsigned-${KERNEL_VER}-generic"

    echo ""
    echo "Downloaded packages:"
    ls -lh *.deb

    echo ""
    echo "Verifying package checksum..."
    ACTUAL_SHA256="$(sha256sum linux-image-unsigned-*.deb | awk '{print $1}')"
    if [[ "$ACTUAL_SHA256" != "$KERNEL_PKG_SHA256" ]]; then
        die "Package checksum mismatch (expected $KERNEL_PKG_SHA256, got $ACTUAL_SHA256)"
    fi
    echo "[OK] Package checksum verified: $ACTUAL_SHA256"

    mkdir -p extracted
    run_cmd dpkg-deb -x linux-image-unsigned-*.deb extracted/

    mkdir -p "$DEST"
    run_cmd cp extracted/boot/vmlinuz-* "$DEST/vmlinuz"
    touch -d "@${SOURCE_DATE_EPOCH}" "$DEST/vmlinuz"
popd >/dev/null

echo ""
echo "=========================================="
echo "[OK] Kernel build complete"
echo "=========================================="
echo "Output: $DEST/vmlinuz"
echo "Version: ${KERNEL_VER}"
echo "APT mode: ${APT_SOURCE_MODE}"
echo "SHA256: $(sha256sum "$DEST/vmlinuz" | awk '{print $1}')"
echo "=========================================="
