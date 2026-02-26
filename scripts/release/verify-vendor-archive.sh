#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

VENDOR_DIR="$PROJECT_ROOT/third_party/cargo"
VENDOR_ARCHIVE_NAME="${VENDOR_ARCHIVE_NAME:-vendor.tar.gz}"
VENDOR_PART_GLOB="${VENDOR_PART_GLOB:-$VENDOR_DIR/${VENDOR_ARCHIVE_NAME}.part-*}"
VENDOR_ARCHIVE_SHA256_FILE="${VENDOR_ARCHIVE_SHA256_FILE:-$VENDOR_DIR/vendor.tar.gz.sha256}"
VENDOR_MANIFEST_FILE="${VENDOR_MANIFEST_FILE:-$VENDOR_DIR/VENDOR_MANIFEST.lock}"
CARGO_LOCK_FILE="${CARGO_LOCK_FILE:-$PROJECT_ROOT/Cargo.lock}"

EXTRACT_CHECK=0

error() {
    echo "ERROR: $*" >&2
    exit 1
}

require_cmd() {
    command -v "$1" >/dev/null 2>&1 || error "Missing required command: $1"
}

hash_file() {
    local file="$1"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$file" | awk '{ print $1 }'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$file" | awk '{ print $1 }'
    else
        error "Neither sha256sum nor shasum is available"
    fi
}

parse_manifest_value() {
    local key="$1"
    awk -F'=' -v k="$key" '$1 == k { print substr($0, index($0, "=") + 1) }' "$VENDOR_MANIFEST_FILE" | tail -n 1
}

collect_vendor_parts() {
    VENDOR_PARTS=()
    while IFS= read -r part; do
        VENDOR_PARTS+=("$part")
    done < <(find "$VENDOR_DIR" -maxdepth 1 -type f -name "${VENDOR_ARCHIVE_NAME}.part-*" | LC_ALL=C sort)
    [[ ${#VENDOR_PARTS[@]} -gt 0 ]] || error "No vendor archive parts found for pattern: $VENDOR_PART_GLOB"
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --extract-check)
            EXTRACT_CHECK=1
            shift
            ;;
        -h|--help)
            cat <<EOF
Usage: $0 [--extract-check]

Verifies:
  - vendor archive checksum
  - manifest checksum consistency
  - Cargo.lock checksum matches manifest
  - optional archive extraction sanity
EOF
            exit 0
            ;;
        *)
            error "Unknown option: $1"
            ;;
    esac
done

require_cmd tar
[[ -f "$VENDOR_ARCHIVE_SHA256_FILE" ]] || error "Vendor archive sha file not found: $VENDOR_ARCHIVE_SHA256_FILE"
[[ -f "$VENDOR_MANIFEST_FILE" ]] || error "Vendor manifest not found: $VENDOR_MANIFEST_FILE"
[[ -f "$CARGO_LOCK_FILE" ]] || error "Cargo.lock not found: $CARGO_LOCK_FILE"
collect_vendor_parts

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT
TMP_ARCHIVE="$TMP_DIR/$VENDOR_ARCHIVE_NAME"
cat "${VENDOR_PARTS[@]}" > "$TMP_ARCHIVE"

EXPECTED_ARCHIVE_SHA="$(awk '{ print $1 }' "$VENDOR_ARCHIVE_SHA256_FILE" | head -n 1)"
[[ -n "$EXPECTED_ARCHIVE_SHA" ]] || error "Vendor archive sha file is empty: $VENDOR_ARCHIVE_SHA256_FILE"

ACTUAL_ARCHIVE_SHA="$(hash_file "$TMP_ARCHIVE")"
if [[ "$EXPECTED_ARCHIVE_SHA" != "$ACTUAL_ARCHIVE_SHA" ]]; then
    error "Vendor archive checksum mismatch. expected=$EXPECTED_ARCHIVE_SHA actual=$ACTUAL_ARCHIVE_SHA"
fi

MANIFEST_ARCHIVE_SHA="$(parse_manifest_value vendor_archive_sha256)"
[[ -n "$MANIFEST_ARCHIVE_SHA" ]] || error "Missing vendor_archive_sha256 in $VENDOR_MANIFEST_FILE"
if [[ "$MANIFEST_ARCHIVE_SHA" != "$ACTUAL_ARCHIVE_SHA" ]]; then
    error "Manifest vendor archive sha mismatch. expected=$MANIFEST_ARCHIVE_SHA actual=$ACTUAL_ARCHIVE_SHA"
fi

MANIFEST_LOCK_SHA="$(parse_manifest_value cargo_lock_sha256)"
[[ -n "$MANIFEST_LOCK_SHA" ]] || error "Missing cargo_lock_sha256 in $VENDOR_MANIFEST_FILE"

ACTUAL_LOCK_SHA="$(hash_file "$CARGO_LOCK_FILE")"
if [[ "$MANIFEST_LOCK_SHA" != "$ACTUAL_LOCK_SHA" ]]; then
    error "Cargo.lock checksum mismatch. expected=$MANIFEST_LOCK_SHA actual=$ACTUAL_LOCK_SHA"
fi

if [[ $EXTRACT_CHECK -eq 1 ]]; then
    EXTRACT_DIR="$TMP_DIR/extracted"
    mkdir -p "$EXTRACT_DIR"
    tar -xzf "$TMP_ARCHIVE" -C "$EXTRACT_DIR"
    [[ -d "$EXTRACT_DIR/cargo-home" ]] || error "Vendor archive extract check failed: missing cargo-home directory"
    [[ -d "$EXTRACT_DIR/cargo-home/registry/cache" ]] || error "Vendor archive extract check failed: missing registry/cache"
    [[ -d "$EXTRACT_DIR/cargo-home/git/db" ]] || error "Vendor archive extract check failed: missing git/db"

    if [[ -z "$(find "$EXTRACT_DIR/cargo-home/registry/cache" -type f -print -quit)" ]]; then
        error "Vendor archive extract check failed: no cached registry crates found"
    fi
fi

echo "Vendor archive verification succeeded."
