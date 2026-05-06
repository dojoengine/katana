#!/usr/bin/env bash
#
# Build and package a reproducible Linux amd64 Katana release artifact.
#
# This script contains the reproducible release build flow so it can be run
# outside GitHub Actions as long as the host has Docker and GNU release tooling.
#
# Usage:
#   ./scripts/build-reproducible-katana.sh --version v1.7.0
#   SOURCE_DATE_EPOCH=$(git log -1 --format=%ct) ./scripts/build-reproducible-katana.sh --version v1.7.0

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

VERSION_NAME="${VERSION_NAME:-}"
SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-}"
OUTPUT_DIR="${OUTPUT_DIR:-dist/reproducible-katana}"
DOCKERFILE="${DOCKERFILE:-reproducible.Dockerfile}"
CONTEXT_DIR="${CONTEXT_DIR:-$PROJECT_ROOT}"
ARCHIVE_NAME="${ARCHIVE_NAME:-}"
PACKAGE_DIR_NAME=""
IMAGE_PREFIX="${IMAGE_PREFIX:-katana-reproducible}"
PLATFORM="${PLATFORM:-linux/amd64}"
PASSES=2
NO_CACHE=1
RUST_IMAGE="${RUST_IMAGE:-}"

usage() {
    cat <<'EOF'
Usage: scripts/build-reproducible-katana.sh [OPTIONS]

Build Katana in the pinned reproducible Docker environment, compare repeated
build outputs byte-for-byte, and package a deterministic Linux amd64 artifact.

OPTIONS:
  --version VERSION             Release version/tag, e.g. v1.7.0
  --source-date-epoch EPOCH     Reproducible timestamp. Defaults to HEAD commit time
  --output-dir DIR              Output directory. Defaults to dist/reproducible-katana
  --archive-name NAME           Archive file name. Defaults to katana_VERSION_linux_amd64.tar.gz
  --package-dir NAME            Optional directory to place inside the archive
  --dockerfile PATH             Dockerfile path. Defaults to reproducible.Dockerfile
  --context DIR                 Docker build context. Defaults to the repository root
  --image-prefix NAME           Docker image tag prefix. Defaults to katana-reproducible
  --platform PLATFORM           Docker target platform. Defaults to linux/amd64
  --passes COUNT                Number of clean builds to run and compare. Defaults to 2
  --rust-image IMAGE            Override the Dockerfile RUST_IMAGE build arg
  --use-cache                   Allow Docker layer cache instead of --no-cache
  -h, --help                    Show this help
EOF
}

die() {
    echo "ERROR: $*" >&2
    exit 1
}

require_command() {
    if ! command -v "$1" >/dev/null 2>&1; then
        die "Required command not found: $1"
    fi
}

require_arg() {
    if [[ $# -lt 2 || -z "${2:-}" ]]; then
        die "Missing value for $1"
    fi
}

version_from_cargo_toml() {
    awk '
        $0 == "[workspace.package]" { in_workspace_package = 1; next }
        in_workspace_package && /^\[/ { in_workspace_package = 0 }
        in_workspace_package && /^version[[:space:]]*=/ {
            gsub(/"/, "", $3)
            print $3
            exit
        }
    ' "$PROJECT_ROOT/Cargo.toml"
}

shell_escape() {
    printf "%q" "$1"
}

sanitize_tag() {
    local tag
    tag="$(printf "%s" "$1" | tr -c '[:alnum:]_.-' '-' | sed 's/^-*//; s/-*$//')"
    if [[ -z "$tag" ]]; then
        tag="local"
    fi
    printf "%s" "$tag"
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --version)
            require_arg "$@"
            VERSION_NAME="$2"
            shift 2
            ;;
        --source-date-epoch)
            require_arg "$@"
            SOURCE_DATE_EPOCH="$2"
            shift 2
            ;;
        --output-dir)
            require_arg "$@"
            OUTPUT_DIR="$2"
            shift 2
            ;;
        --archive-name)
            require_arg "$@"
            ARCHIVE_NAME="$2"
            shift 2
            ;;
        --package-dir)
            require_arg "$@"
            PACKAGE_DIR_NAME="$2"
            shift 2
            ;;
        --dockerfile)
            require_arg "$@"
            DOCKERFILE="$2"
            shift 2
            ;;
        --context)
            require_arg "$@"
            CONTEXT_DIR="$2"
            shift 2
            ;;
        --image-prefix)
            require_arg "$@"
            IMAGE_PREFIX="$2"
            shift 2
            ;;
        --platform)
            require_arg "$@"
            PLATFORM="$2"
            shift 2
            ;;
        --passes)
            require_arg "$@"
            PASSES="$2"
            shift 2
            ;;
        --rust-image)
            require_arg "$@"
            RUST_IMAGE="$2"
            shift 2
            ;;
        --use-cache)
            NO_CACHE=0
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            die "Unknown option: $1"
            ;;
    esac
done

cd "$PROJECT_ROOT"

if [[ -z "$VERSION_NAME" ]]; then
    CARGO_VERSION="$(version_from_cargo_toml)"
    [[ -n "$CARGO_VERSION" ]] || die "Could not infer version from Cargo.toml; pass --version"
    VERSION_NAME="v${CARGO_VERSION}"
fi

if [[ -z "$SOURCE_DATE_EPOCH" ]]; then
    SOURCE_DATE_EPOCH="$(git log -1 --format=%ct 2>/dev/null || true)"
    [[ -n "$SOURCE_DATE_EPOCH" ]] || die "Could not infer SOURCE_DATE_EPOCH; pass --source-date-epoch"
fi

[[ "$SOURCE_DATE_EPOCH" =~ ^[0-9]+$ ]] || die "SOURCE_DATE_EPOCH must be a Unix timestamp"
[[ "$PASSES" =~ ^[0-9]+$ ]] || die "--passes must be a positive integer"
[[ "$PASSES" -gt 0 ]] || die "--passes must be a positive integer"

if [[ "$DOCKERFILE" != /* ]]; then
    DOCKERFILE="$PROJECT_ROOT/$DOCKERFILE"
fi

if [[ "$CONTEXT_DIR" != /* ]]; then
    CONTEXT_DIR="$PROJECT_ROOT/$CONTEXT_DIR"
fi

[[ -f "$DOCKERFILE" ]] || die "Dockerfile not found: $DOCKERFILE"
[[ -d "$CONTEXT_DIR" ]] || die "Docker build context not found: $CONTEXT_DIR"

require_command docker
require_command awk
require_command cmp
require_command file
require_command gzip
require_command readelf
require_command sed
require_command sha256sum
require_command sha384sum
require_command seq
require_command tar
require_command touch

if ! tar --version 2>/dev/null | grep -q "GNU tar"; then
    die "GNU tar is required for deterministic archive metadata"
fi

FILE_VERSION="$(printf "%s" "$VERSION_NAME" | tr '/' '-')"
if [[ -z "$ARCHIVE_NAME" ]]; then
    ARCHIVE_NAME="katana_${FILE_VERSION}_linux_amd64.tar.gz"
fi

mkdir -p "$OUTPUT_DIR"
OUTPUT_DIR="$(cd "$OUTPUT_DIR" && pwd)"
WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/katana-reproducible.XXXXXX")"
CONTAINERS=()

cleanup() {
    local container
    for container in "${CONTAINERS[@]:-}"; do
        docker rm -f "$container" >/dev/null 2>&1 || true
    done
    rm -rf "$WORK_DIR"
}
trap cleanup EXIT

IMAGE_TAG="$(sanitize_tag "$VERSION_NAME")"

echo "Building reproducible Katana artifact"
echo "  version: $VERSION_NAME"
echo "  source date epoch: $SOURCE_DATE_EPOCH"
echo "  platform: $PLATFORM"
echo "  passes: $PASSES"
echo "  output dir: $OUTPUT_DIR"

for pass in $(seq 1 "$PASSES"); do
    IMAGE="${IMAGE_PREFIX}:${IMAGE_TAG}-${pass}"
    CONTAINER="katana-extract-${pass}-$$"
    BUILD_ARGS=(
        build
        -f "$DOCKERFILE"
        --platform "$PLATFORM"
        -t "$IMAGE"
        --build-arg "SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}"
    )

    if [[ -n "$RUST_IMAGE" ]]; then
        BUILD_ARGS+=(--build-arg "RUST_IMAGE=${RUST_IMAGE}")
    fi

    if [[ "$NO_CACHE" -eq 1 ]]; then
        BUILD_ARGS+=(--no-cache)
    fi

    BUILD_ARGS+=("$CONTEXT_DIR")

    echo ""
    echo "Docker build pass $pass/$PASSES: $IMAGE"
    docker "${BUILD_ARGS[@]}"

    CONTAINERS+=("$CONTAINER")
    docker create --name "$CONTAINER" "$IMAGE" >/dev/null
    docker cp "$CONTAINER:/katana" "$WORK_DIR/katana-${pass}"
    docker cp "$CONTAINER:/katana.build-info" "$WORK_DIR/katana-${pass}.build-info"
    docker cp "$CONTAINER:/katana.sha256" "$WORK_DIR/katana-${pass}.sha256"
    docker cp "$CONTAINER:/katana.sha384" "$WORK_DIR/katana-${pass}.sha384"
    docker rm "$CONTAINER" >/dev/null

    sha256sum "$WORK_DIR/katana-${pass}"
    sha384sum "$WORK_DIR/katana-${pass}"
done

if [[ "$PASSES" -gt 1 ]]; then
    for pass in $(seq 2 "$PASSES"); do
        cmp "$WORK_DIR/katana-1" "$WORK_DIR/katana-${pass}"
        cmp "$WORK_DIR/katana-1.build-info" "$WORK_DIR/katana-${pass}.build-info"
    done
fi

install -m 0755 "$WORK_DIR/katana-1" "$OUTPUT_DIR/katana"
cp "$WORK_DIR/katana-1.build-info" "$OUTPUT_DIR/build-info.txt"
touch -d "@${SOURCE_DATE_EPOCH}" "$OUTPUT_DIR/katana" "$OUTPUT_DIR/build-info.txt"

echo ""
echo "Inspecting binary"
file "$OUTPUT_DIR/katana"
readelf -l "$OUTPUT_DIR/katana" | grep 'Requesting program interpreter'
readelf -d "$OUTPUT_DIR/katana" | grep 'NEEDED'

PACKAGE_WORK_DIR="$WORK_DIR/package"
mkdir -p "$PACKAGE_WORK_DIR"

if [[ -n "$PACKAGE_DIR_NAME" ]]; then
    mkdir -p "$PACKAGE_WORK_DIR/$PACKAGE_DIR_NAME"
    cp "$OUTPUT_DIR/katana" "$PACKAGE_WORK_DIR/$PACKAGE_DIR_NAME/katana"
    cp "$OUTPUT_DIR/build-info.txt" "$PACKAGE_WORK_DIR/$PACKAGE_DIR_NAME/build-info.txt"
    chmod 0755 "$PACKAGE_WORK_DIR/$PACKAGE_DIR_NAME/katana"
    touch -d "@${SOURCE_DATE_EPOCH}" \
        "$PACKAGE_WORK_DIR/$PACKAGE_DIR_NAME" \
        "$PACKAGE_WORK_DIR/$PACKAGE_DIR_NAME/katana" \
        "$PACKAGE_WORK_DIR/$PACKAGE_DIR_NAME/build-info.txt"
    TAR_INPUTS=("$PACKAGE_DIR_NAME")
else
    cp "$OUTPUT_DIR/katana" "$PACKAGE_WORK_DIR/katana"
    cp "$OUTPUT_DIR/build-info.txt" "$PACKAGE_WORK_DIR/build-info.txt"
    chmod 0755 "$PACKAGE_WORK_DIR/katana"
    touch -d "@${SOURCE_DATE_EPOCH}" \
        "$PACKAGE_WORK_DIR/katana" \
        "$PACKAGE_WORK_DIR/build-info.txt"
    TAR_INPUTS=(build-info.txt katana)
fi

ARCHIVE_PATH="$OUTPUT_DIR/$ARCHIVE_NAME"
tar \
    --sort=name \
    --owner=0 \
    --group=0 \
    --numeric-owner \
    --mtime="@${SOURCE_DATE_EPOCH}" \
    -cf - \
    -C "$PACKAGE_WORK_DIR" \
    "${TAR_INPUTS[@]}" | gzip -n > "$ARCHIVE_PATH"
touch -d "@${SOURCE_DATE_EPOCH}" "$ARCHIVE_PATH"

BINARY_SHA256="$(
    cd "$OUTPUT_DIR"
    sha256sum katana | tee katana.sha256 | awk '{ print $1 }'
)"
BINARY_SHA384="$(
    cd "$OUTPUT_DIR"
    sha384sum katana | tee katana.sha384 | awk '{ print $1 }'
)"
ARCHIVE_SHA256="$(
    cd "$OUTPUT_DIR"
    sha256sum "$ARCHIVE_NAME" | tee "${ARCHIVE_NAME}.sha256" | awk '{ print $1 }'
)"
ARCHIVE_SHA384="$(
    cd "$OUTPUT_DIR"
    sha384sum "$ARCHIVE_NAME" | tee "${ARCHIVE_NAME}.sha384" | awk '{ print $1 }'
)"
touch -d "@${SOURCE_DATE_EPOCH}" \
    "$OUTPUT_DIR/katana.sha256" \
    "$OUTPUT_DIR/katana.sha384" \
    "$OUTPUT_DIR/${ARCHIVE_NAME}.sha256" \
    "$OUTPUT_DIR/${ARCHIVE_NAME}.sha384"

MANIFEST_PATH="$OUTPUT_DIR/manifest.env"
{
    printf "VERSION_NAME=%s\n" "$(shell_escape "$VERSION_NAME")"
    printf "SOURCE_DATE_EPOCH=%s\n" "$(shell_escape "$SOURCE_DATE_EPOCH")"
    printf "BINARY_PATH=%s\n" "$(shell_escape "$OUTPUT_DIR/katana")"
    printf "BUILD_INFO_PATH=%s\n" "$(shell_escape "$OUTPUT_DIR/build-info.txt")"
    printf "ARCHIVE_NAME=%s\n" "$(shell_escape "$ARCHIVE_NAME")"
    printf "ARCHIVE_PATH=%s\n" "$(shell_escape "$ARCHIVE_PATH")"
    printf "BINARY_SHA256=%s\n" "$(shell_escape "$BINARY_SHA256")"
    printf "BINARY_SHA384=%s\n" "$(shell_escape "$BINARY_SHA384")"
    printf "ARCHIVE_SHA256=%s\n" "$(shell_escape "$ARCHIVE_SHA256")"
    printf "ARCHIVE_SHA384=%s\n" "$(shell_escape "$ARCHIVE_SHA384")"
} > "$MANIFEST_PATH"

echo ""
echo "Reproducible Katana artifact built"
echo "  binary: $OUTPUT_DIR/katana"
echo "  archive: $ARCHIVE_PATH"
echo "  manifest: $MANIFEST_PATH"
echo "  binary sha256: $BINARY_SHA256"
echo "  archive sha256: $ARCHIVE_SHA256"
