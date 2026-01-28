#!/bin/bash
#
# Build TEE components (OVMF, kernel, initrd) for AMD SEV-SNP.
# This script should be run from the repository root directory.
#
# Usage:
#   ./misc/AMDSEV/build.sh
#   ./misc/AMDSEV/build.sh --katana /path/to/katana
#   ./misc/AMDSEV/build.sh ovmf kernel
#

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. ${SCRIPT_DIR}/build-config

# Export variables for child scripts
export OVMF_GIT_URL OVMF_BRANCH KERNEL_VERSION
export KERNEL_PKG_SHA256 BUSYBOX_PKG_SHA256 KERNEL_MODULES_EXTRA_PKG_SHA256

# Set SOURCE_DATE_EPOCH if not already set (for reproducible builds)
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(date +%s)}"

function usage()
{
	echo "Usage: $0 [OPTIONS] [COMPONENTS]"
	echo ""
	echo "OPTIONS:"
	echo "  --install PATH          Installation path (default: ${SCRIPT_DIR}/output/qemu)"
	echo "  --katana PATH           Path to katana binary (optional, will build if not provided)"
	echo "  -h|--help               Usage information"
	echo ""
	echo "COMPONENTS (if none specified, builds all):"
	echo "  ovmf                    Build OVMF firmware"
	echo "  kernel                  Build kernel"
	echo "  initrd                  Build initrd (builds katana if --katana not provided)"

	exit 1
}

INSTALL_DIR="${SCRIPT_DIR}/output/qemu"
KATANA_BINARY=""
BUILD_OVMF=0
BUILD_KERNEL=0
BUILD_INITRD=0

while [ -n "$1" ]; do
	case "$1" in
	--install)
		[ -z "$2" ] && usage
		INSTALL_DIR="$2"
		shift; shift
		;;
	--katana)
		[ -z "$2" ] && usage
		KATANA_BINARY="$2"
		shift; shift
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
		echo "Unsupported option: [$1]"
		usage
		;;
	*)
		echo "Unsupported argument: [$1]"
		usage
		;;
	esac
done

# If no components specified, build all
if [ $BUILD_OVMF -eq 0 ] && [ $BUILD_KERNEL -eq 0 ] && [ $BUILD_INITRD -eq 0 ]; then
	BUILD_OVMF=1
	BUILD_KERNEL=1
	BUILD_INITRD=1
fi

# Build katana if needed for initrd and not provided
if [ $BUILD_INITRD -eq 1 ] && [ -z "$KATANA_BINARY" ]; then
	echo "No --katana provided, building katana with musl..."
	PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
	"${PROJECT_ROOT}/scripts/build-musl.sh"
	if [ $? -ne 0 ]; then
		echo "Katana build failed"
		exit 1
	fi
	KATANA_BINARY="${PROJECT_ROOT}/target/x86_64-unknown-linux-musl/performance/katana"
	if [ ! -f "$KATANA_BINARY" ]; then
		echo "ERROR: Katana binary not found at $KATANA_BINARY"
		exit 1
	fi
	echo "Using built katana: $KATANA_BINARY"
fi

mkdir -p $INSTALL_DIR
IDIR=$INSTALL_DIR
INSTALL_DIR=$(readlink -e $INSTALL_DIR)
[ -n "$INSTALL_DIR" -a -d "$INSTALL_DIR" ] || {
	echo "Installation directory [$IDIR] does not exist, exiting"
	exit 1
}

if [ $BUILD_OVMF -eq 1 ]; then
	"${SCRIPT_DIR}/build-ovmf.sh" "$INSTALL_DIR"
	if [ $? -ne 0 ]; then
		echo "OVMF build failed: $?"
		exit 1
	fi
fi

if [ $BUILD_KERNEL -eq 1 ]; then
	"${SCRIPT_DIR}/build-kernel.sh" "$INSTALL_DIR"
	if [ $? -ne 0 ]; then
		echo "Kernel build failed: $?"
		exit 1
	fi
fi

if [ $BUILD_INITRD -eq 1 ]; then
	"${SCRIPT_DIR}/build-initrd.sh" "$KATANA_BINARY" "$INSTALL_DIR/initrd.img"
	if [ $? -ne 0 ]; then
		echo "Initrd build failed: $?"
		exit 1
	fi
	# Copy katana binary to output directory
	cp "$KATANA_BINARY" "$INSTALL_DIR/katana"
	echo "Copied katana binary to $INSTALL_DIR/katana"
fi

# ==============================================================================
# Generate build-info.txt (merge with existing if present)
# ==============================================================================
BUILD_INFO="$INSTALL_DIR/build-info.txt"

# Initialize variables with defaults (empty)
INFO_OVMF_GIT_URL=""
INFO_OVMF_BRANCH=""
INFO_OVMF_COMMIT=""
INFO_KERNEL_VERSION=""
INFO_KERNEL_PKG_SHA256=""
INFO_BUSYBOX_PKG_SHA256=""
INFO_KERNEL_MODULES_EXTRA_PKG_SHA256=""
INFO_KATANA_BINARY_SHA256=""
INFO_OVMF_SHA256=""
INFO_KERNEL_SHA256=""
INFO_INITRD_SHA256=""

# Load existing values if build-info.txt exists
if [ -f "$BUILD_INFO" ]; then
	while IFS='=' read -r key value; do
		# Skip comments and empty lines
		[[ "$key" =~ ^#.*$ || -z "$key" ]] && continue
		case "$key" in
			OVMF_GIT_URL) INFO_OVMF_GIT_URL="$value" ;;
			OVMF_BRANCH) INFO_OVMF_BRANCH="$value" ;;
			OVMF_COMMIT) INFO_OVMF_COMMIT="$value" ;;
			KERNEL_VERSION) INFO_KERNEL_VERSION="$value" ;;
			KERNEL_PKG_SHA256) INFO_KERNEL_PKG_SHA256="$value" ;;
			BUSYBOX_PKG_SHA256) INFO_BUSYBOX_PKG_SHA256="$value" ;;
			KERNEL_MODULES_EXTRA_PKG_SHA256) INFO_KERNEL_MODULES_EXTRA_PKG_SHA256="$value" ;;
			KATANA_BINARY_SHA256) INFO_KATANA_BINARY_SHA256="$value" ;;
			OVMF_SHA256) INFO_OVMF_SHA256="$value" ;;
			KERNEL_SHA256) INFO_KERNEL_SHA256="$value" ;;
			INITRD_SHA256) INFO_INITRD_SHA256="$value" ;;
		esac
	done < "$BUILD_INFO"
fi

# Update values for components that were built
if [ $BUILD_OVMF -eq 1 ]; then
	INFO_OVMF_GIT_URL="$OVMF_GIT_URL"
	INFO_OVMF_BRANCH="$OVMF_BRANCH"
	[ -f "${SCRIPT_DIR}/source-commit.ovmf" ] && INFO_OVMF_COMMIT="$(cat "${SCRIPT_DIR}/source-commit.ovmf")"
	[ -f "$INSTALL_DIR/OVMF.fd" ] && INFO_OVMF_SHA256="$(sha256sum "$INSTALL_DIR/OVMF.fd" | awk '{print $1}')"
fi

if [ $BUILD_KERNEL -eq 1 ]; then
	INFO_KERNEL_VERSION="$KERNEL_VERSION"
	INFO_KERNEL_PKG_SHA256="$KERNEL_PKG_SHA256"
	[ -f "$INSTALL_DIR/vmlinuz" ] && INFO_KERNEL_SHA256="$(sha256sum "$INSTALL_DIR/vmlinuz" | awk '{print $1}')"
fi

if [ $BUILD_INITRD -eq 1 ]; then
	INFO_BUSYBOX_PKG_SHA256="$BUSYBOX_PKG_SHA256"
	INFO_KERNEL_MODULES_EXTRA_PKG_SHA256="$KERNEL_MODULES_EXTRA_PKG_SHA256"
	[ -n "$KATANA_BINARY" ] && [ -f "$KATANA_BINARY" ] && INFO_KATANA_BINARY_SHA256="$(sha256sum "$KATANA_BINARY" | awk '{print $1}')"
	[ -f "$INSTALL_DIR/initrd.img" ] && INFO_INITRD_SHA256="$(sha256sum "$INSTALL_DIR/initrd.img" | awk '{print $1}')"
fi

# Write build-info.txt with all values
{
	echo "# TEE Build Information"
	echo "# Generated: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
	echo ""
	echo "# Reproducibility"
	echo "SOURCE_DATE_EPOCH=$SOURCE_DATE_EPOCH"
	echo ""
	echo "# Dependencies"
	[ -n "$INFO_OVMF_GIT_URL" ] && echo "OVMF_GIT_URL=$INFO_OVMF_GIT_URL"
	[ -n "$INFO_OVMF_BRANCH" ] && echo "OVMF_BRANCH=$INFO_OVMF_BRANCH"
	[ -n "$INFO_OVMF_COMMIT" ] && echo "OVMF_COMMIT=$INFO_OVMF_COMMIT"
	[ -n "$INFO_KERNEL_VERSION" ] && echo "KERNEL_VERSION=$INFO_KERNEL_VERSION"
	[ -n "$INFO_KERNEL_PKG_SHA256" ] && echo "KERNEL_PKG_SHA256=$INFO_KERNEL_PKG_SHA256"
	[ -n "$INFO_BUSYBOX_PKG_SHA256" ] && echo "BUSYBOX_PKG_SHA256=$INFO_BUSYBOX_PKG_SHA256"
	[ -n "$INFO_KERNEL_MODULES_EXTRA_PKG_SHA256" ] && echo "KERNEL_MODULES_EXTRA_PKG_SHA256=$INFO_KERNEL_MODULES_EXTRA_PKG_SHA256"
	[ -n "$INFO_KATANA_BINARY_SHA256" ] && echo "KATANA_BINARY_SHA256=$INFO_KATANA_BINARY_SHA256"
	echo ""
	echo "# Output Checksums (SHA256)"
	[ -n "$INFO_OVMF_SHA256" ] && echo "OVMF_SHA256=$INFO_OVMF_SHA256"
	[ -n "$INFO_KERNEL_SHA256" ] && echo "KERNEL_SHA256=$INFO_KERNEL_SHA256"
	[ -n "$INFO_INITRD_SHA256" ] && echo "INITRD_SHA256=$INFO_INITRD_SHA256"
} > "$BUILD_INFO"

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
