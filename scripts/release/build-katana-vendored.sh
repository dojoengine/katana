#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

TARGET=""
PROFILE="${PROFILE:-performance}"
NATIVE_BUILD=0

VENDOR_DIR="${VENDOR_DIR:-$PROJECT_ROOT/third_party/cargo}"
VENDOR_ARCHIVE_NAME="${VENDOR_ARCHIVE_NAME:-vendor.tar.gz}"
VERIFY_SCRIPT="$SCRIPT_DIR/verify-vendor-archive.sh"

error() {
    echo "ERROR: $*" >&2
    exit 1
}

require_cmd() {
    command -v "$1" >/dev/null 2>&1 || error "Missing required command: $1"
}

collect_vendor_parts() {
    VENDOR_PARTS=()
    while IFS= read -r part; do
        VENDOR_PARTS+=("$part")
    done < <(find "$VENDOR_DIR" -maxdepth 1 -type f -name "${VENDOR_ARCHIVE_NAME}.part-*" | LC_ALL=C sort)
    [[ ${#VENDOR_PARTS[@]} -gt 0 ]] || error "No vendor archive parts found in $VENDOR_DIR"
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --target)
            [[ -n "${2:-}" ]] || error "Missing value for --target"
            TARGET="$2"
            shift 2
            ;;
        --profile)
            [[ -n "${2:-}" ]] || error "Missing value for --profile"
            PROFILE="$2"
            shift 2
            ;;
        --native)
            NATIVE_BUILD=1
            shift
            ;;
        -h|--help)
            cat <<EOF
Usage: $0 --target <rust-target> [--profile <cargo-profile>] [--native]

Builds katana with:
  - vendored dependencies only
  - --locked --offline --frozen cargo mode
EOF
            exit 0
            ;;
        *)
            error "Unknown option: $1"
            ;;
    esac
done

[[ -n "$TARGET" ]] || error "target is required (use --target)"

require_cmd cargo
require_cmd tar
[[ -x "$VERIFY_SCRIPT" ]] || error "Verification script is not executable: $VERIFY_SCRIPT"
collect_vendor_parts

if [[ -z "${SOURCE_DATE_EPOCH:-}" ]]; then
    error "SOURCE_DATE_EPOCH must be set"
fi
if ! [[ "$SOURCE_DATE_EPOCH" =~ ^[0-9]+$ ]]; then
    error "SOURCE_DATE_EPOCH must be an integer unix timestamp"
fi

"$VERIFY_SCRIPT"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

EXTRACT_DIR="$TMP_DIR/extracted"
mkdir -p "$EXTRACT_DIR"
TMP_ARCHIVE="$TMP_DIR/$VENDOR_ARCHIVE_NAME"
cat "${VENDOR_PARTS[@]}" > "$TMP_ARCHIVE"

tar -xzf "$TMP_ARCHIVE" -C "$EXTRACT_DIR"
[[ -d "$EXTRACT_DIR/cargo-home" ]] || error "Vendor archive missing top-level cargo-home/ directory"
CARGO_HOME_DIR="$EXTRACT_DIR/cargo-home"

FEATURE_ARGS=()
if [[ $NATIVE_BUILD -eq 1 ]]; then
    FEATURE_ARGS=(--features native)
fi

echo "Building katana (target=$TARGET, profile=$PROFILE, native=$NATIVE_BUILD)"
CARGO_HOME="$CARGO_HOME_DIR" cargo build \
    --locked \
    --offline \
    --frozen \
    -p katana \
    --bin katana \
    --profile "$PROFILE" \
    --target "$TARGET" \
    "${FEATURE_ARGS[@]}"

BINARY_PATH="$PROJECT_ROOT/target/$TARGET/$PROFILE/katana"
if [[ "$TARGET" == *"windows"* ]]; then
    BINARY_PATH="$PROJECT_ROOT/target/$TARGET/$PROFILE/katana.exe"
fi

[[ -f "$BINARY_PATH" ]] || error "Expected binary not found at $BINARY_PATH"
echo "Build succeeded: $BINARY_PATH"
