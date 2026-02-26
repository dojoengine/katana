#!/bin/bash
#
# Build TEE components (OVMF, kernel, initrd) for AMD SEV-SNP.
# This script should be run from the repository root directory.
#
# Usage:
#   ./misc/AMDSEV/build.sh
#   ./misc/AMDSEV/build.sh --katana /path/to/katana
#   ./misc/AMDSEV/build.sh --repro-check ovmf kernel initrd
#

set -euo pipefail

# Environment normalization for reproducibility.
export TZ=UTC
export LANG=C.UTF-8
export LC_ALL=C.UTF-8
umask 022

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
. "${SCRIPT_DIR}/build-config"

# Export variables for child scripts.
export OVMF_GIT_URL OVMF_BRANCH OVMF_COMMIT KERNEL_VERSION
export KERNEL_PKG_SHA256 BUSYBOX_PKG_SHA256 KERNEL_MODULES_EXTRA_PKG_SHA256
export BUSYBOX_PKG_VERSION KERNEL_MODULES_EXTRA_PKG_VERSION
export APT_SNAPSHOT_URL APT_SNAPSHOT_SUITE APT_SNAPSHOT_COMPONENTS
export BUILD_CONTAINER_IMAGE_DIGEST
export KATANA_STRICT_REPRO

usage() {
	echo "Usage: $0 [OPTIONS] [COMPONENTS]"
	echo ""
	echo "OPTIONS:"
	echo "  --install PATH          Installation path (default: ${SCRIPT_DIR}/output/qemu)"
	echo "  --katana PATH           Path to katana binary (optional, will build if not provided)"
	echo "  --repro-check           Build twice and fail if output hashes differ"
	echo "  -h|--help               Usage information"
	echo ""
	echo "COMPONENTS (if none specified, builds all):"
	echo "  ovmf                    Build OVMF firmware"
	echo "  kernel                  Build kernel"
	echo "  initrd                  Build initrd (builds katana if --katana not provided)"
	exit 1
}

die() {
	echo "ERROR: $*" >&2
	exit 1
}

require_source_date_epoch() {
	if [[ -z "${SOURCE_DATE_EPOCH:-}" ]]; then
		die "SOURCE_DATE_EPOCH must be set for reproducible builds (e.g. export SOURCE_DATE_EPOCH=\$(git log -1 --format=%ct))"
	fi
	if ! [[ "$SOURCE_DATE_EPOCH" =~ ^[0-9]+$ ]]; then
		die "SOURCE_DATE_EPOCH must be a unix timestamp integer"
	fi
}

tool_version() {
	local cmd="$1"
	if command -v "$cmd" >/dev/null 2>&1; then
		"$cmd" --version 2>/dev/null | head -n 1 | tr -s ' '
	else
		echo "${cmd}:not-installed"
	fi
}

INSTALL_DIR="${SCRIPT_DIR}/output/qemu"
KATANA_BINARY=""
BUILD_OVMF=0
BUILD_KERNEL=0
BUILD_INITRD=0
REPRO_CHECK=0

while [[ $# -gt 0 ]]; do
	case "$1" in
	--install)
		[[ -z "${2:-}" ]] && usage
		INSTALL_DIR="$2"
		shift 2
		;;
	--katana)
		[[ -z "${2:-}" ]] && usage
		KATANA_BINARY="$2"
		shift 2
		;;
	--repro-check)
		REPRO_CHECK=1
		shift
		;;
	-h|--help)
		usage
		;;
	ovmf)
		BUILD_OVMF=1
		shift
		;;
	kernel)
		BUILD_KERNEL=1
		shift
		;;
	initrd)
		BUILD_INITRD=1
		shift
		;;
	-*|--*)
		die "Unsupported option: [$1]"
		;;
	*)
		die "Unsupported argument: [$1]"
		;;
	esac
done

# If no components specified, build all.
if [[ $BUILD_OVMF -eq 0 && $BUILD_KERNEL -eq 0 && $BUILD_INITRD -eq 0 ]]; then
	BUILD_OVMF=1
	BUILD_KERNEL=1
	BUILD_INITRD=1
fi

require_source_date_epoch

echo ""
if [[ -z "${OVMF_COMMIT:-}" ]]; then
	die "OVMF_COMMIT must be pinned in build-config"
fi
if [[ -z "${APT_SNAPSHOT_URL:-}" ]]; then
	echo "WARNING: APT_SNAPSHOT_URL is not set; package resolution depends on host apt sources"
	echo "         Set APT_SNAPSHOT_URL/APT_SNAPSHOT_SUITE/APT_SNAPSHOT_COMPONENTS for stronger reproducibility"
fi
echo ""

# Build katana if needed for initrd and not provided.
if [[ $BUILD_INITRD -eq 1 && -z "$KATANA_BINARY" ]]; then
	echo "No --katana provided."
	if [[ ! -t 0 ]]; then
		die "Cannot prompt without an interactive terminal. Pass --katana /path/to/katana to use a pre-built binary."
	fi

	read -r -p "Build katana from source with musl now? [y/N] " CONFIRM_BUILD_KATANA
	case "$CONFIRM_BUILD_KATANA" in
		[yY]|[yY][eE][sS])
			echo "Building katana with musl..."
			;;
		*)
			die "Aborting. Provide --katana /path/to/katana to use a pre-built binary."
			;;
		esac

		PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
		if [[ "${KATANA_STRICT_REPRO:-0}" == "1" ]]; then
			"${PROJECT_ROOT}/scripts/build-musl.sh" --strict
		else
			echo "WARNING: Building katana without --strict dependency vendoring."
			echo "         Set KATANA_STRICT_REPRO=1 (and vendor deps) for stronger reproducibility."
			"${PROJECT_ROOT}/scripts/build-musl.sh"
		fi
		KATANA_BINARY="${PROJECT_ROOT}/target/x86_64-unknown-linux-musl/performance/katana"
		[[ -f "$KATANA_BINARY" ]] || die "Katana binary not found at $KATANA_BINARY"
		echo "Using built katana: $KATANA_BINARY"
	fi

if [[ -n "$KATANA_BINARY" ]]; then
	KATANA_BINARY="$(readlink -e "$KATANA_BINARY")"
fi

mkdir -p "$INSTALL_DIR"
IDIR="$INSTALL_DIR"
INSTALL_DIR="$(readlink -e "$INSTALL_DIR")"
[[ -n "$INSTALL_DIR" && -d "$INSTALL_DIR" ]] || die "Installation directory [$IDIR] does not exist"

if [[ $BUILD_OVMF -eq 1 ]]; then
	"${SCRIPT_DIR}/build-ovmf.sh" "$INSTALL_DIR"
fi

if [[ $BUILD_KERNEL -eq 1 ]]; then
	"${SCRIPT_DIR}/build-kernel.sh" "$INSTALL_DIR"
fi

if [[ $BUILD_INITRD -eq 1 ]]; then
	"${SCRIPT_DIR}/build-initrd.sh" "$KATANA_BINARY" "$INSTALL_DIR/initrd.img"
	cp "$KATANA_BINARY" "$INSTALL_DIR/katana"
	echo "Copied katana binary to $INSTALL_DIR/katana"
fi

BUILD_INFO="$INSTALL_DIR/build-info.txt"
MATERIALS_LOCK="$INSTALL_DIR/materials.lock"

INFO_OVMF_COMMIT="$OVMF_COMMIT"
[[ -f "${SCRIPT_DIR}/source-commit.ovmf" ]] && INFO_OVMF_COMMIT="$(cat "${SCRIPT_DIR}/source-commit.ovmf")"

INFO_OVMF_SHA256=""
INFO_KERNEL_SHA256=""
INFO_INITRD_SHA256=""
INFO_KATANA_BINARY_SHA256=""

[[ -f "$INSTALL_DIR/OVMF.fd" ]] && INFO_OVMF_SHA256="$(sha256sum "$INSTALL_DIR/OVMF.fd" | awk '{print $1}')"
[[ -f "$INSTALL_DIR/vmlinuz" ]] && INFO_KERNEL_SHA256="$(sha256sum "$INSTALL_DIR/vmlinuz" | awk '{print $1}')"
[[ -f "$INSTALL_DIR/initrd.img" ]] && INFO_INITRD_SHA256="$(sha256sum "$INSTALL_DIR/initrd.img" | awk '{print $1}')"
if [[ -f "$INSTALL_DIR/katana" ]]; then
	INFO_KATANA_BINARY_SHA256="$(sha256sum "$INSTALL_DIR/katana" | awk '{print $1}')"
elif [[ -n "$KATANA_BINARY" && -f "$KATANA_BINARY" ]]; then
	INFO_KATANA_BINARY_SHA256="$(sha256sum "$KATANA_BINARY" | awk '{print $1}')"
fi

TOOLCHAIN_ID="bash=${BASH_VERSION};$(tool_version gcc);$(tool_version ld);$(tool_version cpio);$(tool_version gzip);$(tool_version dpkg-deb);$(tool_version apt-get)"

INPUT_MANIFEST_SHA256="$({
	echo "SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}"
	echo "OVMF_GIT_URL=${OVMF_GIT_URL}"
	echo "OVMF_BRANCH=${OVMF_BRANCH}"
	echo "OVMF_COMMIT=${INFO_OVMF_COMMIT}"
	echo "KERNEL_VERSION=${KERNEL_VERSION}"
	echo "KERNEL_PKG_SHA256=${KERNEL_PKG_SHA256}"
	echo "BUSYBOX_PKG_VERSION=${BUSYBOX_PKG_VERSION}"
	echo "BUSYBOX_PKG_SHA256=${BUSYBOX_PKG_SHA256}"
	echo "KERNEL_MODULES_EXTRA_PKG_VERSION=${KERNEL_MODULES_EXTRA_PKG_VERSION}"
	echo "KERNEL_MODULES_EXTRA_PKG_SHA256=${KERNEL_MODULES_EXTRA_PKG_SHA256}"
	echo "APT_SNAPSHOT_URL=${APT_SNAPSHOT_URL:-}"
	echo "APT_SNAPSHOT_SUITE=${APT_SNAPSHOT_SUITE:-}"
	echo "APT_SNAPSHOT_COMPONENTS=${APT_SNAPSHOT_COMPONENTS:-}"
	echo "BUILD_CONTAINER_IMAGE_DIGEST=${BUILD_CONTAINER_IMAGE_DIGEST:-}"
	echo "KATANA_STRICT_REPRO=${KATANA_STRICT_REPRO:-0}"
} | sha256sum | awk '{print $1}')"

{
	echo "# TEE Build Information"
	echo ""
	echo "# Reproducibility"
	echo "SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}"
	echo "INPUT_MANIFEST_SHA256=${INPUT_MANIFEST_SHA256}"
	echo "TOOLCHAIN_ID=${TOOLCHAIN_ID}"
	echo "BUILD_CONTAINER_IMAGE_DIGEST=${BUILD_CONTAINER_IMAGE_DIGEST:-}"
	echo ""
	echo "# Dependencies"
	echo "OVMF_GIT_URL=${OVMF_GIT_URL}"
	echo "OVMF_BRANCH=${OVMF_BRANCH}"
	echo "OVMF_COMMIT=${INFO_OVMF_COMMIT}"
	echo "KERNEL_VERSION=${KERNEL_VERSION}"
	echo "KERNEL_PKG_SHA256=${KERNEL_PKG_SHA256}"
	echo "BUSYBOX_PKG_VERSION=${BUSYBOX_PKG_VERSION}"
	echo "BUSYBOX_PKG_SHA256=${BUSYBOX_PKG_SHA256}"
	echo "KERNEL_MODULES_EXTRA_PKG_VERSION=${KERNEL_MODULES_EXTRA_PKG_VERSION}"
	echo "KERNEL_MODULES_EXTRA_PKG_SHA256=${KERNEL_MODULES_EXTRA_PKG_SHA256}"
	echo "APT_SNAPSHOT_URL=${APT_SNAPSHOT_URL:-}"
	echo "APT_SNAPSHOT_SUITE=${APT_SNAPSHOT_SUITE:-}"
	echo "APT_SNAPSHOT_COMPONENTS=${APT_SNAPSHOT_COMPONENTS:-}"
	echo "BUILD_CONTAINER_IMAGE_DIGEST=${BUILD_CONTAINER_IMAGE_DIGEST:-}"
	echo "KATANA_STRICT_REPRO=${KATANA_STRICT_REPRO:-0}"
	[[ -n "$INFO_KATANA_BINARY_SHA256" ]] && echo "KATANA_BINARY_SHA256=${INFO_KATANA_BINARY_SHA256}"
	echo ""
	echo "# Output Checksums (SHA256)"
	[[ -n "$INFO_OVMF_SHA256" ]] && echo "OVMF_SHA256=${INFO_OVMF_SHA256}"
	[[ -n "$INFO_KERNEL_SHA256" ]] && echo "KERNEL_SHA256=${INFO_KERNEL_SHA256}"
	[[ -n "$INFO_INITRD_SHA256" ]] && echo "INITRD_SHA256=${INFO_INITRD_SHA256}"
} > "$BUILD_INFO"

{
	echo "# Immutable Build Materials"
	echo "SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}"
	echo "INPUT_MANIFEST_SHA256=${INPUT_MANIFEST_SHA256}"
	echo "OVMF_GIT_URL=${OVMF_GIT_URL}"
	echo "OVMF_BRANCH=${OVMF_BRANCH}"
	echo "OVMF_COMMIT=${INFO_OVMF_COMMIT}"
	echo "KERNEL_VERSION=${KERNEL_VERSION}"
	echo "KERNEL_PKG_SHA256=${KERNEL_PKG_SHA256}"
	echo "BUSYBOX_PKG_VERSION=${BUSYBOX_PKG_VERSION}"
	echo "BUSYBOX_PKG_SHA256=${BUSYBOX_PKG_SHA256}"
	echo "KERNEL_MODULES_EXTRA_PKG_VERSION=${KERNEL_MODULES_EXTRA_PKG_VERSION}"
	echo "KERNEL_MODULES_EXTRA_PKG_SHA256=${KERNEL_MODULES_EXTRA_PKG_SHA256}"
	echo "APT_SNAPSHOT_URL=${APT_SNAPSHOT_URL:-}"
	echo "APT_SNAPSHOT_SUITE=${APT_SNAPSHOT_SUITE:-}"
	echo "APT_SNAPSHOT_COMPONENTS=${APT_SNAPSHOT_COMPONENTS:-}"
	echo "BUILD_CONTAINER_IMAGE_DIGEST=${BUILD_CONTAINER_IMAGE_DIGEST:-}"
	echo "KATANA_STRICT_REPRO=${KATANA_STRICT_REPRO:-0}"
	[[ -n "$INFO_OVMF_SHA256" ]] && echo "ARTIFACT_OVMF_SHA256=${INFO_OVMF_SHA256}"
	[[ -n "$INFO_KERNEL_SHA256" ]] && echo "ARTIFACT_KERNEL_SHA256=${INFO_KERNEL_SHA256}"
	[[ -n "$INFO_INITRD_SHA256" ]] && echo "ARTIFACT_INITRD_SHA256=${INFO_INITRD_SHA256}"
	[[ -n "$INFO_KATANA_BINARY_SHA256" ]] && echo "ARTIFACT_KATANA_SHA256=${INFO_KATANA_BINARY_SHA256}"
} > "$MATERIALS_LOCK"

touch -d "@${SOURCE_DATE_EPOCH}" "$BUILD_INFO" "$MATERIALS_LOCK"

if [[ $REPRO_CHECK -eq 1 && -z "${REPRO_CHECK_INTERNAL:-}" ]]; then
	COMPARE_DIR="$(mktemp -d)"
	cleanup_compare() {
		if [[ -d "$COMPARE_DIR" ]]; then
			rm -rf "$COMPARE_DIR"
		fi
	}
	trap cleanup_compare EXIT INT TERM

	echo ""
	echo "=========================================="
	echo "Reproducibility check"
	echo "=========================================="
	echo "Second build output: $COMPARE_DIR"

	REBUILD_ARGS=(--install "$COMPARE_DIR")
	[[ -n "$KATANA_BINARY" ]] && REBUILD_ARGS+=(--katana "$KATANA_BINARY")
	[[ $BUILD_OVMF -eq 1 ]] && REBUILD_ARGS+=(ovmf)
	[[ $BUILD_KERNEL -eq 1 ]] && REBUILD_ARGS+=(kernel)
	[[ $BUILD_INITRD -eq 1 ]] && REBUILD_ARGS+=(initrd)

	REPRO_CHECK_INTERNAL=1 SOURCE_DATE_EPOCH="$SOURCE_DATE_EPOCH" \
		"${SCRIPT_DIR}/build.sh" "${REBUILD_ARGS[@]}"

	"${SCRIPT_DIR}/verify-build.sh" --compare "$INSTALL_DIR" "$COMPARE_DIR"
	echo "[OK] Reproducibility check passed"
fi

echo ""
echo "=========================================="
echo "Build complete"
echo "=========================================="
echo "Output directory: $INSTALL_DIR"
echo ""
ls -lh "$INSTALL_DIR"
echo ""
echo "Build info:"
cat "$BUILD_INFO"
echo "=========================================="
