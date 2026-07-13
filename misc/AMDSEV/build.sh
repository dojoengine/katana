#!/bin/bash
#
# Build TEE components (OVMF, kernel, initrd) for AMD SEV-SNP.
#
# Usage:
#   ./build.sh --katana /path/to/katana
#   ./build.sh --katana /path/to/katana ovmf kernel
#
# A prebuilt katana binary is required (--katana). Download from:
#   https://github.com/dojoengine/katana/releases
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
# CA certificates bundle — consumed by build-initrd.sh to populate the enclave trust
# stores (openssl + rustls paths). Without this export the cert step silently skips
# and the released image ships NO CA bundle (outbound HTTPS fails).
export CA_CERTIFICATES_PKG_VERSION CA_CERTIFICATES_PKG_SHA256

# SOURCE_DATE_EPOCH controls timestamps embedded in OVMF and the initrd cpio
# archive — directly affects launch-measurement reproducibility. If unset, fall
# back to the current wall clock and surface a loud warning so the caller knows
# the resulting measurement is tied to "when this happened to run".
if [[ -z "${SOURCE_DATE_EPOCH:-}" ]]; then
    SOURCE_DATE_EPOCH="$(date +%s)"
    export SOURCE_DATE_EPOCH
    # GNU date uses `-d @SECS`, BSD/macOS uses `-r SECS`. Try both.
    SOURCE_DATE_EPOCH_HUMAN=$(date -u -d "@${SOURCE_DATE_EPOCH}" +%Y-%m-%dT%H:%M:%SZ 2>/dev/null \
                           || date -u -r "${SOURCE_DATE_EPOCH}" +%Y-%m-%dT%H:%M:%SZ 2>/dev/null \
                           || echo "unknown")
    cat >&2 <<EOF

WARNING: SOURCE_DATE_EPOCH was not set.
         Falling back to the current wall clock:
             SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}  (${SOURCE_DATE_EPOCH_HUMAN})
         The build will embed this timestamp in OVMF and the initrd, so the
         resulting launch measurement will differ from any other build of the
         same source. For a reproducible build, set it explicitly — typically
         from the commit time of the tree you're building:
             export SOURCE_DATE_EPOCH=\$(git log -1 --format=%ct)

EOF
else
    export SOURCE_DATE_EPOCH
fi

if [[ -z "${OVMF_COMMIT:-}" ]]; then
    echo "WARNING: OVMF_COMMIT not set - OVMF build may not be reproducible" >&2
fi

function usage()
{
	echo "Usage: $0 [OPTIONS] [COMPONENTS]"
	echo ""
	echo "OPTIONS:"
	echo "  --install PATH          Installation path (default: ${SCRIPT_DIR}/output/qemu)"
	echo "  --katana PATH           Path to katana binary (required when building initrd)"
	echo "  --snp-derivekey PATH    Path to snp-derivekey binary (optional; auto-built if not"
	echo "                          provided). Required for sealed-mode initrd unless"
	echo "                          KATANA_UNSEALED_BUILD=1 is set."
	echo "  --cryptsetup PATH       Path to a static cryptsetup binary (optional; auto-built"
	echo "                          via build-cryptsetup.sh if not provided). Required for"
	echo "                          sealed-mode initrd."
	echo "  --mkfs-ext2 PATH        Path to a static mkfs.ext2 binary (optional; auto-built"
	echo "                          via build-cryptsetup.sh if not provided). Required for"
	echo "                          sealed-mode initrd."
	echo "  --paymaster-bin PATH    Path to a prebuilt paymaster-service binary (optional;"
	echo "                          katana release asset). Bundled into the initrd so the"
	echo "                          guest supports --paymaster. Release images always"
	echo "                          bundle it. Both-or-neither with --vrf-bin."
	echo "  --vrf-bin PATH          Path to a prebuilt vrf-server binary (optional; katana"
	echo "                          release asset). Bundled into the initrd so the guest"
	echo "                          supports --vrf. Both-or-neither with --paymaster-bin."
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
	--paymaster-bin)
		[ -z "$2" ] && usage
		# Same export rationale as --snp-derivekey: build-initrd.sh runs
		# as a child process and reads PAYMASTER_BINARY from the env. No
		# auto-build fallback — the sidecars are prebuilt katana release
		# assets, not vendored source.
		export PAYMASTER_BINARY="$2"
		shift; shift
		;;
	--vrf-bin)
		[ -z "$2" ] && usage
		export VRF_BINARY="$2"
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
	-*)
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

# A katana binary is required when building the initrd. This standalone repo
# does not vendor katana source, so the build-from-source fallback that lived
# here previously has been removed — operators must supply a prebuilt binary
# with --katana. See dojoengine/katana releases for prebuilt linux-gnu binaries.
if [ $BUILD_INITRD -eq 1 ] && [ -z "$KATANA_BINARY" ]; then
	echo "ERROR: --katana <path-to-katana-binary> is required when building the initrd."
	echo ""
	echo "Download a prebuilt katana binary from:"
	echo "  https://github.com/dojoengine/katana/releases"
	echo "and pass it via --katana, e.g.:"
	echo "  $0 --katana /path/to/katana"
	exit 1
fi

# Build snp-derivekey for the canonical sealed initrd unless the operator
# opted out (KATANA_UNSEALED_BUILD=1) or pre-supplied a binary path. The
# source is vendored in this repo's snp-tools workspace (gated behind the
# `snp-derivekey` feature); built for musl so it ships with no runtime
# libc dependency.
if [ $BUILD_INITRD -eq 1 ] \
   && [ "${KATANA_UNSEALED_BUILD:-0}" -ne 1 ] \
   && [ -z "${SNP_DERIVEKEY_BINARY:-}" ]; then
	SNP_TOOLS_DIR="${SCRIPT_DIR}/snp-tools"
	SNP_DERIVEKEY_BINARY="${SNP_TOOLS_DIR}/target/x86_64-unknown-linux-musl/release/snp-derivekey"
	if [ ! -x "$SNP_DERIVEKEY_BINARY" ]; then
		if ! command -v cargo >/dev/null 2>&1; then
			echo ""
			echo "ERROR: snp-derivekey not found at $SNP_DERIVEKEY_BINARY and cargo is not on PATH."
			echo ""
			echo "If you are running build.sh under sudo, cargo is likely installed under your"
			echo "regular user (\$HOME/.cargo/bin) but not in root's PATH. Two options:"
			echo ""
			echo "  1. Pre-build snp-derivekey as your normal user, then pass the path:"
			echo "       (cd ${SNP_TOOLS_DIR} && cargo build \\"
			echo "          --locked --target x86_64-unknown-linux-musl --release \\"
			echo "          --features snp-derivekey --bin snp-derivekey)"
			echo "       sudo $0 --katana <path> --snp-derivekey \\"
			echo "         $SNP_DERIVEKEY_BINARY ..."
			echo ""
			echo "  2. Run build.sh with sudo -E to inherit your PATH (assumes cargo on it)."
			exit 1
		fi
		echo ""
		echo "Building snp-derivekey with musl (sealed-storage helper)..."
		( cd "$SNP_TOOLS_DIR" && \
		  cargo build \
		    --locked \
		    --target x86_64-unknown-linux-musl \
		    --release \
		    --features snp-derivekey \
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
# $SCRIPT_DIR/output/cryptsetup-static and skip when both binaries are
# already present.
if [ $BUILD_INITRD -eq 1 ] \
   && [ "${KATANA_UNSEALED_BUILD:-0}" -ne 1 ] \
   && { [ -z "${CRYPTSETUP_BINARY:-}" ] || [ -z "${MKFS_EXT2_BINARY:-}" ]; }; then
	CRYPTSETUP_OUT_DIR="${SCRIPT_DIR}/output/cryptsetup-static"
	CRYPTSETUP_BINARY="${CRYPTSETUP_BINARY:-${CRYPTSETUP_OUT_DIR}/cryptsetup}"
	MKFS_EXT2_BINARY="${MKFS_EXT2_BINARY:-${CRYPTSETUP_OUT_DIR}/mkfs.ext2}"

	if [ ! -x "$CRYPTSETUP_BINARY" ] || [ ! -x "$MKFS_EXT2_BINARY" ]; then
		echo ""
		echo "Building static cryptsetup + mkfs.ext2 (sealed-storage helpers)..."
		"${SCRIPT_DIR}/scripts/build-cryptsetup.sh" "$CRYPTSETUP_OUT_DIR" || {
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
[ -n "$INSTALL_DIR" ] && [ -d "$INSTALL_DIR" ] || {
	echo "Installation directory [$IDIR] does not exist, exiting"
	exit 1
}

if [ $BUILD_OVMF -eq 1 ]; then
	"${SCRIPT_DIR}/scripts/build-ovmf.sh" "$INSTALL_DIR"
	rc=$?
	if [ $rc -ne 0 ]; then
		echo "OVMF build failed: $rc"
		exit 1
	fi
fi

if [ $BUILD_KERNEL -eq 1 ]; then
	"${SCRIPT_DIR}/scripts/build-kernel.sh" "$INSTALL_DIR"
	rc=$?
	if [ $rc -ne 0 ]; then
		echo "Kernel build failed: $rc"
		exit 1
	fi
fi

if [ $BUILD_INITRD -eq 1 ]; then
	"${SCRIPT_DIR}/scripts/build-initrd.sh" "$KATANA_BINARY" "$INSTALL_DIR/initrd.img"
	rc=$?
	if [ $rc -ne 0 ]; then
		echo "Initrd build failed: $rc"
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
INFO_OVMF_SOURCE_DATE_EPOCH=""
INFO_KERNEL_VERSION=""
INFO_KERNEL_PKG_SHA256=""
INFO_BUSYBOX_PKG_SHA256=""
INFO_GLIBC_RUNTIME_PACKAGES=""
INFO_GLIBC_RUNTIME_PACKAGE_SHA256S=""
INFO_GLIBC_VERSION=""
INFO_KERNEL_MODULES_EXTRA_PKG_SHA256=""
INFO_KATANA_BINARY_SHA256=""
INFO_PAYMASTER_BINARY_SHA256=""
INFO_VRF_BINARY_SHA256=""
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
			OVMF_SOURCE_DATE_EPOCH) INFO_OVMF_SOURCE_DATE_EPOCH="$value" ;;
			KERNEL_VERSION) INFO_KERNEL_VERSION="$value" ;;
			KERNEL_PKG_SHA256) INFO_KERNEL_PKG_SHA256="$value" ;;
			BUSYBOX_PKG_SHA256) INFO_BUSYBOX_PKG_SHA256="$value" ;;
			GLIBC_RUNTIME_PACKAGES) INFO_GLIBC_RUNTIME_PACKAGES="$value" ;;
			GLIBC_RUNTIME_PACKAGE_SHA256S) INFO_GLIBC_RUNTIME_PACKAGE_SHA256S="$value" ;;
			GLIBC_VERSION) INFO_GLIBC_VERSION="$value" ;;
			KERNEL_MODULES_EXTRA_PKG_SHA256) INFO_KERNEL_MODULES_EXTRA_PKG_SHA256="$value" ;;
			KATANA_BINARY_SHA256) INFO_KATANA_BINARY_SHA256="$value" ;;
			PAYMASTER_BINARY_SHA256) INFO_PAYMASTER_BINARY_SHA256="$value" ;;
			VRF_BINARY_SHA256) INFO_VRF_BINARY_SHA256="$value" ;;
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
	# OVMF_COMMIT is pinned in build-config; that's the authoritative value
	# build-ovmf.sh checked out. Source it directly rather than re-reading a
	# truncated short hash from a side-channel file.
	INFO_OVMF_COMMIT="$OVMF_COMMIT"
	# The epoch the firmware was built with — derived by build-ovmf.sh from
	# the OVMF commit's own timestamp (NOT the release epoch) and dropped
	# alongside the artifact.
	[ -f "$INSTALL_DIR/ovmf-source-date-epoch.txt" ] && INFO_OVMF_SOURCE_DATE_EPOCH="$(cat "$INSTALL_DIR/ovmf-source-date-epoch.txt")"
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
	[ -n "${PAYMASTER_BINARY:-}" ] && [ -f "$PAYMASTER_BINARY" ] && INFO_PAYMASTER_BINARY_SHA256="$(sha256sum "$PAYMASTER_BINARY" | awk '{print $1}')"
	[ -n "${VRF_BINARY:-}" ] && [ -f "$VRF_BINARY" ] && INFO_VRF_BINARY_SHA256="$(sha256sum "$VRF_BINARY" | awk '{print $1}')"
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
	[ -n "$INFO_OVMF_SOURCE_DATE_EPOCH" ] && echo "OVMF_SOURCE_DATE_EPOCH=$INFO_OVMF_SOURCE_DATE_EPOCH"
	[ -n "$INFO_KERNEL_VERSION" ] && echo "KERNEL_VERSION=$INFO_KERNEL_VERSION"
	[ -n "$INFO_KERNEL_PKG_SHA256" ] && echo "KERNEL_PKG_SHA256=$INFO_KERNEL_PKG_SHA256"
	[ -n "$INFO_BUSYBOX_PKG_SHA256" ] && echo "BUSYBOX_PKG_SHA256=$INFO_BUSYBOX_PKG_SHA256"
	[ -n "$INFO_GLIBC_VERSION" ] && echo "GLIBC_VERSION=$INFO_GLIBC_VERSION"
	[ -n "$INFO_GLIBC_RUNTIME_PACKAGES" ] && echo "GLIBC_RUNTIME_PACKAGES=$INFO_GLIBC_RUNTIME_PACKAGES"
	[ -n "$INFO_GLIBC_RUNTIME_PACKAGE_SHA256S" ] && echo "GLIBC_RUNTIME_PACKAGE_SHA256S=$INFO_GLIBC_RUNTIME_PACKAGE_SHA256S"
	[ -n "$INFO_KERNEL_MODULES_EXTRA_PKG_SHA256" ] && echo "KERNEL_MODULES_EXTRA_PKG_SHA256=$INFO_KERNEL_MODULES_EXTRA_PKG_SHA256"
	[ -n "$INFO_KATANA_BINARY_SHA256" ] && echo "KATANA_BINARY_SHA256=$INFO_KATANA_BINARY_SHA256"
	[ -n "$INFO_PAYMASTER_BINARY_SHA256" ] && echo "PAYMASTER_BINARY_SHA256=$INFO_PAYMASTER_BINARY_SHA256"
	[ -n "$INFO_VRF_BINARY_SHA256" ] && echo "VRF_BINARY_SHA256=$INFO_VRF_BINARY_SHA256"
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
