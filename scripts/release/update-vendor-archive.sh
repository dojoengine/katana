#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

VENDOR_DIR="$PROJECT_ROOT/third_party/cargo"
VENDOR_ARCHIVE_NAME="${VENDOR_ARCHIVE_NAME:-vendor.tar.gz}"
VENDOR_PART_PREFIX="${VENDOR_PART_PREFIX:-$VENDOR_DIR/${VENDOR_ARCHIVE_NAME}.part-}"
VENDOR_PART_SIZE="${VENDOR_PART_SIZE:-95m}"
VENDOR_ARCHIVE_SHA256_FILE="${VENDOR_ARCHIVE_SHA256_FILE:-$VENDOR_DIR/vendor.tar.gz.sha256}"
VENDOR_MANIFEST_FILE="${VENDOR_MANIFEST_FILE:-$VENDOR_DIR/VENDOR_MANIFEST.lock}"
CARGO_LOCK_FILE="${CARGO_LOCK_FILE:-$PROJECT_ROOT/Cargo.lock}"
CARGO_HOME_LAYOUT_DIR="cargo-home"
FETCH_TARGETS=(
    "x86_64-unknown-linux-gnu"
    "aarch64-unknown-linux-gnu"
    "aarch64-apple-darwin"
    "x86_64-pc-windows-msvc"
)

# Use a stable default to avoid archive churn across unrelated commits.
SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-0}"

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

resolve_tar_impl() {
    if command -v gtar >/dev/null 2>&1; then
        echo "gnu:gtar"
        return
    fi

    if command -v tar >/dev/null 2>&1; then
        if tar --version 2>/dev/null | grep -q "GNU tar"; then
            echo "gnu:tar"
            return
        fi
        echo "bsd:tar"
        return
    fi

    error "tar is required"
}

create_archive() {
    local impl="$1"
    local tar_cmd="$2"
    local output_archive="$3"

    if [[ "$impl" == "gnu" ]]; then
        "$tar_cmd" \
            --sort=name \
            --mtime="@$SOURCE_DATE_EPOCH" \
            --owner=0 \
            --group=0 \
            --numeric-owner \
            --format=gnu \
            -C "$TMP_DIR" \
            -czf "$output_archive" \
            "$CARGO_HOME_LAYOUT_DIR"
        return
    fi

    local file_list="$TMP_DIR/vendor-files.txt"
    (cd "$TMP_DIR" && find "$CARGO_HOME_LAYOUT_DIR" -print | LC_ALL=C sort > "$file_list")
    (cd "$TMP_DIR" && "$tar_cmd" --uid 0 --gid 0 --numeric-owner --format pax -cf - -T "$file_list" | gzip -n > "$output_archive")
}

epoch_to_touch_timestamp() {
    if date -u -r 0 "+%Y%m%d%H%M.%S" >/dev/null 2>&1; then
        date -u -r "$SOURCE_DATE_EPOCH" "+%Y%m%d%H%M.%S"
    else
        date -u -d "@$SOURCE_DATE_EPOCH" "+%Y%m%d%H%M.%S"
    fi
}

normalize_timestamps() {
    local dir="$1"
    local timestamp
    timestamp="$(epoch_to_touch_timestamp)"
    find "$dir" -exec touch -h -t "$timestamp" {} +
}

if ! [[ "$SOURCE_DATE_EPOCH" =~ ^[0-9]+$ ]]; then
    error "SOURCE_DATE_EPOCH must be an integer unix timestamp"
fi

require_cmd cargo
require_cmd gzip
require_cmd split
TAR_IMPL_WITH_CMD="$(resolve_tar_impl)"
TAR_IMPL="${TAR_IMPL_WITH_CMD%%:*}"
TAR_CMD="${TAR_IMPL_WITH_CMD##*:}"

[[ -f "$CARGO_LOCK_FILE" ]] || error "Cargo.lock not found at $CARGO_LOCK_FILE"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT
TMP_CARGO_HOME_DIR="$TMP_DIR/$CARGO_HOME_LAYOUT_DIR"
TMP_ARCHIVE="$TMP_DIR/$VENDOR_ARCHIVE_NAME"

echo "Fetching dependencies into isolated CARGO_HOME..."
mkdir -p "$TMP_CARGO_HOME_DIR"
for target in "${FETCH_TARGETS[@]}"; do
    echo "  - target: $target"
    CARGO_HOME="$TMP_CARGO_HOME_DIR" cargo fetch --locked --target "$target" --config net.git-fetch-with-cli=false
done

mkdir -p "$VENDOR_DIR"

echo "Pruning non-essential caches..."
rm -rf "$TMP_CARGO_HOME_DIR/registry/src" "$TMP_CARGO_HOME_DIR/git/checkouts"

echo "Normalizing cached dependency timestamps..."
normalize_timestamps "$TMP_CARGO_HOME_DIR"

echo "Creating deterministic archive: $TMP_ARCHIVE"
create_archive "$TAR_IMPL" "$TAR_CMD" "$TMP_ARCHIVE"

VENDOR_ARCHIVE_SHA256="$(hash_file "$TMP_ARCHIVE")"
printf "%s  %s\n" "$VENDOR_ARCHIVE_SHA256" "$VENDOR_ARCHIVE_NAME" > "$VENDOR_ARCHIVE_SHA256_FILE"

echo "Splitting archive into git-safe parts (size=$VENDOR_PART_SIZE)..."
find "$VENDOR_DIR" -maxdepth 1 -type f -name "${VENDOR_ARCHIVE_NAME}.part-*" -delete
split -b "$VENDOR_PART_SIZE" -d -a 3 "$TMP_ARCHIVE" "$VENDOR_PART_PREFIX"
rm -f "$VENDOR_DIR/$VENDOR_ARCHIVE_NAME"

PART_COUNT="$(find "$VENDOR_DIR" -maxdepth 1 -type f -name "${VENDOR_ARCHIVE_NAME}.part-*" | wc -l | tr -d ' ')"
[[ "$PART_COUNT" -gt 0 ]] || error "No archive parts were generated"

CARGO_LOCK_SHA256="$(hash_file "$CARGO_LOCK_FILE")"

cat > "$VENDOR_MANIFEST_FILE" <<EOF
manifest_version=1
cargo_lock_path=Cargo.lock
cargo_lock_sha256=$CARGO_LOCK_SHA256
vendor_archive_path=third_party/cargo/$VENDOR_ARCHIVE_NAME
vendor_archive_sha256=$VENDOR_ARCHIVE_SHA256
vendor_archive_sha256_file=third_party/cargo/$(basename "$VENDOR_ARCHIVE_SHA256_FILE")
vendor_archive_part_prefix=third_party/cargo/${VENDOR_ARCHIVE_NAME}.part-
source_date_epoch=$SOURCE_DATE_EPOCH
generated_by=scripts/release/update-vendor-archive.sh
EOF

echo "Vendor archive updated."
echo "Files to commit:"
echo "  - third_party/cargo/${VENDOR_ARCHIVE_NAME}.part-* ($PART_COUNT files)"
echo "  - third_party/cargo/$(basename "$VENDOR_ARCHIVE_SHA256_FILE")"
echo "  - third_party/cargo/$(basename "$VENDOR_MANIFEST_FILE")"
