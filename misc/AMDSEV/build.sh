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
export OVMF_GIT_URL OVMF_BRANCH OVMF_COMMIT KERNEL_VERSION
export KERNEL_PKG_SHA256 BUSYBOX_PKG_SHA256 KERNEL_MODULES_EXTRA_PKG_SHA256
export BUSYBOX_PKG_VERSION KERNEL_MODULES_EXTRA_PKG_VERSION
# Sealed-storage build pins. KERNEL_MODULES_* is consumed by build-initrd.sh;
# CRYPTSETUP_*, LVM2_*, E2FSPROGS_*, CRYPTSETUP_BUILDER_IMAGE are consumed by
# build-cryptsetup.sh (auto-invoked below when CRYPTSETUP_BINARY/MKFS_EXT2_BINARY
# aren't already supplied). All required unless KATANA_UNSEALED_BUILD=1.
export KERNEL_MODULES_PKG_VERSION KERNEL_MODULES_PKG_SHA256
export CRYPTSETUP_VERSION CRYPTSETUP_SHA256 CRYPTSETUP_BUILDER_IMAGE
export LVM2_VERSION LVM2_SHA256
export E2FSPROGS_VERSION E2FSPROGS_SHA256
export GLIBC_RUNTIME_PACKAGES GLIBC_RUNTIME_PACKAGE_SHA256S

# Set SOURCE_DATE_EPOCH if not already set (for reproducible builds)
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(date +%s)}"

# Reproducibility validation
echo ""
if [[ -z "${OVMF_COMMIT:-}" ]]; then
    echo "WARNING: OVMF_COMMIT not set - OVMF build may not be reproducible"
fi
if [[ -z "${SOURCE_DATE_EPOCH:-}" ]] || [[ "$SOURCE_DATE_EPOCH" == "$(date +%s)" ]]; then
    echo "NOTE: SOURCE_DATE_EPOCH defaulting to current time"
    echo "      For reproducible builds: export SOURCE_DATE_EPOCH=\$(git log -1 --format=%ct)"
fi
echo ""

function usage()
{
	echo "Usage: $0 [OPTIONS] [COMPONENTS]"
	echo ""
	echo "OPTIONS:"
	echo "  --install PATH          Installation path (default: ${SCRIPT_DIR}/output/qemu)"
	echo "  --katana PATH           Path to katana binary (optional; auto-built if not provided)"
	echo "  --snp-derivekey PATH    Path to snp-derivekey binary (optional; auto-built if not"
	echo "                          provided). Required for sealed-mode initrd unless"
	echo "                          KATANA_UNSEALED_BUILD=1 is set."
	echo "  --cryptsetup PATH       Path to a static cryptsetup binary (optional; auto-built"
	echo "                          via build-cryptsetup.sh if not provided). Required for"
	echo "                          sealed-mode initrd."
	echo "  --mkfs-ext2 PATH        Path to a static mkfs.ext2 binary (optional; auto-built"
	echo "                          via build-cryptsetup.sh if not provided). Required for"
	echo "                          sealed-mode initrd."
	echo "  -h|--help               Usage information"
	echo ""
	echo "COMPONENTS (if none specified, builds all):"
	echo "  ovmf                    Build OVMF firmware"
	echo "  kernel                  Build kernel"
	echo "  initrd                  Build initrd (auto-builds glibc katana, snp-derivekey, and"
	echo "                          cryptsetup + mkfs.ext2 if their --... flags / *_BINARY"
	echo "                          env vars are not set)"

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
	--snp-derivekey)
		[ -z "$2" ] && usage
		# Must export so build-initrd.sh (a child process) sees the path.
		# The auto-build branch below already exports; this is for the
		# `--snp-derivekey PATH` short-circuit case.
		export SNP_DERIVEKEY_BINARY="$2"
		shift; shift
		;;
	--cryptsetup)
		[ -z "$2" ] && usage
		# Same export rationale as --snp-derivekey: build-initrd.sh runs
		# as a child process and reads CRYPTSETUP_BINARY from the env.
		export CRYPTSETUP_BINARY="$2"
		shift; shift
		;;
	--mkfs-ext2)
		[ -z "$2" ] && usage
		export MKFS_EXT2_BINARY="$2"
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
	echo "No --katana provided."
	if [ ! -t 0 ]; then
		echo "ERROR: Cannot prompt without an interactive terminal."
		echo "Pass --katana /path/to/katana to use a pre-built binary."
		exit 1
	fi

	read -r -p "Build katana from source with glibc now? [y/N] " CONFIRM_BUILD_KATANA
	case "$CONFIRM_BUILD_KATANA" in
		[yY]|[yY][eE][sS])
			echo "Building katana with glibc..."
			;;
		*)
			echo "Aborting. Provide --katana /path/to/katana to use a pre-built binary."
			exit 1
			;;
	esac

	PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
	if ! command -v cargo >/dev/null 2>&1; then
		echo ""
		echo "ERROR: cargo is not on PATH."
		echo ""
		echo "If you are running build.sh under sudo, cargo is likely installed under your"
		echo "regular user (\$HOME/.cargo/bin) but not in root's PATH. Two options:"
		echo ""
		echo "  1. Pre-build katana as your normal user, then pass the path:"
		echo "       ${PROJECT_ROOT}/scripts/build-gnu.sh"
		echo "       sudo $0 --katana \\"
		echo "         ${PROJECT_ROOT}/target/x86_64-unknown-linux-gnu/performance/katana ..."
		echo ""
		echo "  2. Run build.sh with sudo -E to inherit your PATH (assumes cargo on it)."
		echo ""
		echo "If cargo is genuinely not installed, set it up via rustup: https://rustup.rs"
		exit 1
	fi
	"${PROJECT_ROOT}/scripts/build-gnu.sh"
	if [ $? -ne 0 ]; then
		echo "Katana build failed"
		exit 1
	fi
	KATANA_BINARY="${PROJECT_ROOT}/target/x86_64-unknown-linux-gnu/performance/katana"
	if [ ! -f "$KATANA_BINARY" ]; then
		echo "ERROR: Katana binary not found at $KATANA_BINARY"
		exit 1
	fi
	echo "Using built katana: $KATANA_BINARY"
fi

# Build snp-derivekey for the canonical sealed initrd unless the operator
# opted out (KATANA_UNSEALED_BUILD=1) or pre-supplied a binary path. Mirrors
# the auto-katana flow above; reuses the workspace's musl target so it ships
# with no runtime libc dependency.
if [ $BUILD_INITRD -eq 1 ] \
   && [ "${KATANA_UNSEALED_BUILD:-0}" -ne 1 ] \
   && [ -z "${SNP_DERIVEKEY_BINARY:-}" ]; then
	PROJECT_ROOT="${PROJECT_ROOT:-$(cd "${SCRIPT_DIR}/../.." && pwd)}"
	SNP_DERIVEKEY_BINARY="${PROJECT_ROOT}/target/x86_64-unknown-linux-musl/performance/snp-derivekey"
	if [ ! -x "$SNP_DERIVEKEY_BINARY" ]; then
		if ! command -v cargo >/dev/null 2>&1; then
			echo ""
			echo "ERROR: snp-derivekey not found at $SNP_DERIVEKEY_BINARY and cargo is not on PATH."
			echo ""
			echo "If you are running build.sh under sudo, cargo is likely installed under your"
			echo "regular user (\$HOME/.cargo/bin) but not in root's PATH. Two options:"
			echo ""
			echo "  1. Pre-build snp-derivekey as your normal user, then pass the path:"
			echo "       cargo build --target x86_64-unknown-linux-musl --profile performance \\"
			echo "         -p katana-tee --features snp --bin snp-derivekey"
			echo "       sudo $0 --katana <path> --snp-derivekey \\"
			echo "         $SNP_DERIVEKEY_BINARY ..."
			echo ""
			echo "  2. Run build.sh with sudo -E to inherit your PATH (assumes cargo on it)."
			exit 1
		fi
		echo ""
		echo "Building snp-derivekey with musl (sealed-storage helper)..."
		( cd "$PROJECT_ROOT" && \
		  cargo build \
		    --locked \
		    --target x86_64-unknown-linux-musl \
		    --profile performance \
		    -p katana-tee --features snp \
		    --bin snp-derivekey ) || {
			echo "snp-derivekey build failed"
			exit 1
		}
	fi
	if [ ! -x "$SNP_DERIVEKEY_BINARY" ]; then
		echo "ERROR: snp-derivekey binary missing at $SNP_DERIVEKEY_BINARY"
		exit 1
	fi
	export SNP_DERIVEKEY_BINARY
	echo "Using snp-derivekey: $SNP_DERIVEKEY_BINARY"
fi

# Build static cryptsetup + mkfs.ext2 for the canonical sealed initrd unless
# the operator opted out (KATANA_UNSEALED_BUILD=1) or pre-supplied both
# binary paths. The container build is non-trivial (~2-3 minutes the first
# time apk-add fetches its mirror), so we cache outputs under
# $PROJECT_ROOT/target/cryptsetup-static and skip when both binaries are
# already present.
if [ $BUILD_INITRD -eq 1 ] \
   && [ "${KATANA_UNSEALED_BUILD:-0}" -ne 1 ] \
   && { [ -z "${CRYPTSETUP_BINARY:-}" ] || [ -z "${MKFS_EXT2_BINARY:-}" ]; }; then
	PROJECT_ROOT="${PROJECT_ROOT:-$(cd "${SCRIPT_DIR}/../.." && pwd)}"
	CRYPTSETUP_OUT_DIR="${PROJECT_ROOT}/target/cryptsetup-static"
	CRYPTSETUP_BINARY="${CRYPTSETUP_BINARY:-${CRYPTSETUP_OUT_DIR}/cryptsetup}"
	MKFS_EXT2_BINARY="${MKFS_EXT2_BINARY:-${CRYPTSETUP_OUT_DIR}/mkfs.ext2}"

	if [ ! -x "$CRYPTSETUP_BINARY" ] || [ ! -x "$MKFS_EXT2_BINARY" ]; then
		echo ""
		echo "Building static cryptsetup + mkfs.ext2 (sealed-storage helpers)..."
		"${SCRIPT_DIR}/build-cryptsetup.sh" "$CRYPTSETUP_OUT_DIR" || {
			echo "build-cryptsetup.sh failed"
			exit 1
		}
	fi
	if [ ! -x "$CRYPTSETUP_BINARY" ]; then
		echo "ERROR: cryptsetup binary missing at $CRYPTSETUP_BINARY"
		exit 1
	fi
	if [ ! -x "$MKFS_EXT2_BINARY" ]; then
		echo "ERROR: mkfs.ext2 binary missing at $MKFS_EXT2_BINARY"
		exit 1
	fi
	export CRYPTSETUP_BINARY MKFS_EXT2_BINARY
	echo "Using cryptsetup: $CRYPTSETUP_BINARY"
	echo "Using mkfs.ext2:  $MKFS_EXT2_BINARY"
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
INFO_GLIBC_RUNTIME_PACKAGES=""
INFO_GLIBC_RUNTIME_PACKAGE_SHA256S=""
INFO_GLIBC_VERSION=""
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
			GLIBC_RUNTIME_PACKAGES) INFO_GLIBC_RUNTIME_PACKAGES="$value" ;;
			GLIBC_RUNTIME_PACKAGE_SHA256S) INFO_GLIBC_RUNTIME_PACKAGE_SHA256S="$value" ;;
			GLIBC_VERSION) INFO_GLIBC_VERSION="$value" ;;
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
	INFO_GLIBC_RUNTIME_PACKAGES="$GLIBC_RUNTIME_PACKAGES"
	INFO_GLIBC_RUNTIME_PACKAGE_SHA256S="$GLIBC_RUNTIME_PACKAGE_SHA256S"
	[ -f "$INSTALL_DIR/glibc-version.txt" ] && INFO_GLIBC_VERSION="$(cat "$INSTALL_DIR/glibc-version.txt")"
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
	[ -n "$INFO_GLIBC_VERSION" ] && echo "GLIBC_VERSION=$INFO_GLIBC_VERSION"
	[ -n "$INFO_GLIBC_RUNTIME_PACKAGES" ] && echo "GLIBC_RUNTIME_PACKAGES=$INFO_GLIBC_RUNTIME_PACKAGES"
	[ -n "$INFO_GLIBC_RUNTIME_PACKAGE_SHA256S" ] && echo "GLIBC_RUNTIME_PACKAGE_SHA256S=$INFO_GLIBC_RUNTIME_PACKAGE_SHA256S"
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
