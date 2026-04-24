#!/bin/bash
# ==============================================================================
# BUILD-INITRD.SH
# ==============================================================================
#
# This script downloads all required dependencies and builds a minimal initrd
# for running Katana inside AMD SEV-SNP confidential VMs.
#
# Dependencies downloaded:
#   - busybox-static: Provides shell and basic utilities
#   - linux-modules:       Contains device-mapper modules (dm-mod.ko, dm-crypt.ko,
#                          dm-integrity.ko) for LUKS-based sealed storage
#   - linux-modules-extra: Contains SEV-SNP kernel modules (tsm.ko, sev-guest.ko)
#   - cryptsetup (source): Built statically inside a pinned Alpine container
#                          for LUKS2 sealed-storage unlock in the measured initrd
#
# Usage:
#   ./build-initrd.sh KATANA_BINARY OUTPUT_INITRD [KERNEL_VERSION]
#
# Environment:
#   SOURCE_DATE_EPOCH                REQUIRED. Unix timestamp for reproducible builds.
#   BUSYBOX_PKG_VERSION              REQUIRED. Exact apt package version (e.g., 1:1.36.1-6ubuntu3.1).
#   BUSYBOX_PKG_SHA256               REQUIRED. SHA256 checksum of the busybox .deb package.
#   KERNEL_MODULES_EXTRA_PKG_VERSION REQUIRED. Exact apt package version.
#   KERNEL_MODULES_EXTRA_PKG_SHA256  REQUIRED. SHA256 checksum of the modules .deb package.
#
# ==============================================================================

set -euo pipefail

REQUIRED_APPLETS=(sh mount umount sleep kill cat mkdir ln mknod ip insmod poweroff sync \
                   tr grep rm blkid mkfifo mkfs.ext2)
SYMLINK_APPLETS=(sh mount umount mkdir mknod switch_root ip insmod sleep kill cat ln poweroff sync \
                  tr grep rm blkid mkfifo mkfs.ext2)

usage() {
    echo "Usage: $0 KATANA_BINARY OUTPUT_INITRD [KERNEL_VERSION]"
    echo ""
    echo "Self-contained initrd builder for Katana TEE VM with AMD SEV-SNP support."
    echo "Downloads all required dependencies (busybox, kernel modules) automatically."
    echo ""
    echo "ARGUMENTS:"
    echo "  KATANA_BINARY    Path to the katana binary (statically linked recommended)"
    echo "  OUTPUT_INITRD    Output path for the generated initrd.img"
    echo "  KERNEL_VERSION   Kernel version for module lookup (or set KERNEL_VERSION env var)"
    echo ""
    echo "ENVIRONMENT VARIABLES (all required for reproducible builds):"
    echo "  SOURCE_DATE_EPOCH                Unix timestamp for reproducible builds"
    echo "  BUSYBOX_PKG_VERSION              Exact apt package version (e.g., 1:1.36.1-6ubuntu3.1)"
    echo "  BUSYBOX_PKG_SHA256               SHA256 checksum of the busybox .deb package"
    echo "  KERNEL_MODULES_PKG_VERSION       Exact apt package version for linux-modules"
    echo "  KERNEL_MODULES_PKG_SHA256        SHA256 checksum of the linux-modules .deb"
    echo "  KERNEL_MODULES_EXTRA_PKG_VERSION Exact apt package version for linux-modules-extra"
    echo "  KERNEL_MODULES_EXTRA_PKG_SHA256  SHA256 checksum of the linux-modules-extra .deb"
    echo "  CRYPTSETUP_VERSION               Exact cryptsetup source release (e.g., 2.7.5)"
    echo "  CRYPTSETUP_SHA256                SHA256 checksum of the cryptsetup source tarball"
    echo "  CRYPTSETUP_BUILDER_IMAGE         Pinned container image digest used to build"
    echo "                                   cryptsetup statically (e.g., alpine@sha256:...)"
    echo ""
    echo "OPTIONAL ENVIRONMENT VARIABLES:"
    echo "  CRYPTSETUP_BUILDER               Container runtime to use (default: docker;"
    echo "                                   can be set to podman or another compatible CLI)"
    echo ""
    echo "EXAMPLES:"
    echo "  export SOURCE_DATE_EPOCH=\$(date +%s)"
    echo "  export BUSYBOX_PKG_VERSION='1:1.36.1-6ubuntu3.1'"
    echo "  export BUSYBOX_PKG_SHA256='abc123...'"
    echo "  export KERNEL_MODULES_PKG_VERSION='6.8.0-90.99'"
    echo "  export KERNEL_MODULES_PKG_SHA256='aaa111...'"
    echo "  export KERNEL_MODULES_EXTRA_PKG_VERSION='6.8.0-90.99'"
    echo "  export KERNEL_MODULES_EXTRA_PKG_SHA256='def456...'"
    echo "  export CRYPTSETUP_VERSION='2.7.5'"
    echo "  export CRYPTSETUP_SHA256='ghi789...'"
    echo "  export CRYPTSETUP_BUILDER_IMAGE='alpine@sha256:jkl012...'"
    echo "  $0 ./katana ./initrd.img 6.8.0-90"
    exit 1
}

log_section() {
    echo ""
    echo "=========================================="
    echo "$*"
    echo "=========================================="
}

log_info() {
    echo "  [INFO] $*"
}

log_ok() {
    echo "  [OK] $*"
}

log_warn() {
    echo "  [WARN] $*"
}

die() {
    echo "ERROR: $*" >&2
    exit 1
}

# Show help if requested or insufficient arguments
if [[ $# -lt 2 ]] || [[ "${1:-}" == "-h" ]] || [[ "${1:-}" == "--help" ]]; then
    usage
fi

to_abs_path() {
    local path="$1"
    if [[ "$path" = /* ]]; then
        printf '%s\n' "$path"
    else
        printf '%s/%s\n' "$(pwd -P)" "$path"
    fi
}

KATANA_BINARY="$(to_abs_path "$1")"
OUTPUT_INITRD="$(to_abs_path "$2")"
KERNEL_VERSION="${3:-${KERNEL_VERSION:?KERNEL_VERSION must be set or passed as third argument}}"
OUTPUT_DIR="$(dirname "$OUTPUT_INITRD")"

log_section "Building Initrd"
echo "Configuration:"
echo "  Katana binary:         $KATANA_BINARY"
echo "  Output initrd:         $OUTPUT_INITRD"
echo "  Kernel version:        $KERNEL_VERSION"
echo "  SOURCE_DATE_EPOCH:     ${SOURCE_DATE_EPOCH:-<not set>}"
echo ""
echo "Package versions:"
echo "  busybox-static:        ${BUSYBOX_PKG_VERSION:-<not set>}"
echo "  linux-modules:         ${KERNEL_MODULES_PKG_VERSION:-<not set>}"
echo "  linux-modules-extra:   ${KERNEL_MODULES_EXTRA_PKG_VERSION:-<not set>}"
echo "  cryptsetup (source):   ${CRYPTSETUP_VERSION:-<not set>}"
echo "  cryptsetup builder:    ${CRYPTSETUP_BUILDER_IMAGE:-<not set>}"

if [[ -z "${SOURCE_DATE_EPOCH:-}" ]]; then
    die "SOURCE_DATE_EPOCH must be set for reproducible builds"
fi

if [[ ! -f "$KATANA_BINARY" ]]; then
    die "Katana binary not found: $KATANA_BINARY"
fi
if [[ ! -x "$KATANA_BINARY" ]]; then
    die "Katana binary is not executable: $KATANA_BINARY"
fi

if [[ ! -d "$OUTPUT_DIR" ]]; then
    mkdir -p "$OUTPUT_DIR" || die "Cannot create output directory: $OUTPUT_DIR"
fi
if [[ ! -w "$OUTPUT_DIR" ]]; then
    die "Output directory is not writable: $OUTPUT_DIR"
fi

REQUIRED_TOOLS=(apt-get dpkg-deb sha256sum cpio gzip zstd find sort touch du mktemp awk grep tr curl tar)
for tool in "${REQUIRED_TOOLS[@]}"; do
    command -v "$tool" >/dev/null 2>&1 || die "Required tool not found: $tool"
done

# Static cryptsetup is built inside a pinned container. Verify the chosen
# runtime is installed now so we fail fast, not after an hour of downloading.
CRYPTSETUP_BUILDER="${CRYPTSETUP_BUILDER:-docker}"
command -v "$CRYPTSETUP_BUILDER" >/dev/null 2>&1 \
    || die "Container runtime '$CRYPTSETUP_BUILDER' not found. Install docker/podman or set CRYPTSETUP_BUILDER."

log_ok "Preflight validation complete"

WORK_DIR="$(mktemp -d)"
cleanup() {
    local exit_code=$?
    if [[ -d "$WORK_DIR" ]]; then
        rm -rf "$WORK_DIR"
    fi
    exit "$exit_code"
}
trap cleanup EXIT INT TERM

log_info "Working directory: $WORK_DIR"

# ==============================================================================
# SECTION 1: Download Required Packages
# ==============================================================================

log_section "Download Required Packages"
PACKAGES_DIR="$WORK_DIR/packages"
mkdir -p "$PACKAGES_DIR"

pushd "$PACKAGES_DIR" >/dev/null

: "${BUSYBOX_PKG_VERSION:?BUSYBOX_PKG_VERSION not set - required for reproducible builds}"
: "${KERNEL_MODULES_PKG_VERSION:?KERNEL_MODULES_PKG_VERSION not set - required for reproducible builds}"
: "${KERNEL_MODULES_EXTRA_PKG_VERSION:?KERNEL_MODULES_EXTRA_PKG_VERSION not set - required for reproducible builds}"

log_info "Downloading busybox-static=${BUSYBOX_PKG_VERSION}"
apt-get download "busybox-static=${BUSYBOX_PKG_VERSION}"

log_info "Downloading linux-modules-${KERNEL_VERSION}-generic=${KERNEL_MODULES_PKG_VERSION}"
apt-get download "linux-modules-${KERNEL_VERSION}-generic=${KERNEL_MODULES_PKG_VERSION}"

log_info "Downloading linux-modules-extra-${KERNEL_VERSION}-generic=${KERNEL_MODULES_EXTRA_PKG_VERSION}"
apt-get download "linux-modules-extra-${KERNEL_VERSION}-generic=${KERNEL_MODULES_EXTRA_PKG_VERSION}"

echo ""
echo "Downloaded packages:"
ls -lh *.deb

: "${BUSYBOX_PKG_SHA256:?BUSYBOX_PKG_SHA256 not set - required for reproducible builds}"
: "${KERNEL_MODULES_PKG_SHA256:?KERNEL_MODULES_PKG_SHA256 not set - required for reproducible builds}"
: "${KERNEL_MODULES_EXTRA_PKG_SHA256:?KERNEL_MODULES_EXTRA_PKG_SHA256 not set - required for reproducible builds}"

log_info "Verifying busybox-static checksum"
ACTUAL_SHA256="$(sha256sum busybox-static_*.deb | awk '{print $1}')"
if [[ "$ACTUAL_SHA256" != "$BUSYBOX_PKG_SHA256" ]]; then
    die "busybox-static checksum mismatch (expected $BUSYBOX_PKG_SHA256, got $ACTUAL_SHA256)"
fi
log_ok "busybox-static checksum verified"

# linux-modules-*.deb and linux-modules-extra-*.deb share the `linux-modules-*`
# filename prefix, so the glob has to be tight enough to pick exactly one file.
log_info "Verifying linux-modules checksum"
MODULES_DEB="$(ls linux-modules-"${KERNEL_VERSION}"-generic_*.deb)"
ACTUAL_SHA256="$(sha256sum "$MODULES_DEB" | awk '{print $1}')"
if [[ "$ACTUAL_SHA256" != "$KERNEL_MODULES_PKG_SHA256" ]]; then
    die "linux-modules checksum mismatch (expected $KERNEL_MODULES_PKG_SHA256, got $ACTUAL_SHA256)"
fi
log_ok "linux-modules checksum verified"

log_info "Verifying linux-modules-extra checksum"
ACTUAL_SHA256="$(sha256sum linux-modules-extra-*.deb | awk '{print $1}')"
if [[ "$ACTUAL_SHA256" != "$KERNEL_MODULES_EXTRA_PKG_SHA256" ]]; then
    die "linux-modules-extra checksum mismatch (expected $KERNEL_MODULES_EXTRA_PKG_SHA256, got $ACTUAL_SHA256)"
fi
log_ok "linux-modules-extra checksum verified"

popd >/dev/null

# ==============================================================================
# SECTION 2: Extract Packages
# ==============================================================================

log_section "Extract Packages"
EXTRACTED_DIR="$WORK_DIR/extracted"
mkdir -p "$EXTRACTED_DIR"

log_info "Extracting busybox-static"
dpkg-deb -x "$PACKAGES_DIR"/busybox-static_*.deb "$EXTRACTED_DIR"

log_info "Extracting linux-modules"
dpkg-deb -x "$PACKAGES_DIR"/linux-modules-"${KERNEL_VERSION}"-generic_*.deb "$EXTRACTED_DIR"

log_info "Extracting linux-modules-extra"
dpkg-deb -x "$PACKAGES_DIR"/linux-modules-extra-*.deb "$EXTRACTED_DIR"
log_ok "Packages extracted"

# ==============================================================================
# SECTION 3: Build Static cryptsetup
# ==============================================================================
# Build a statically-linked cryptsetup binary inside a pinned Alpine
# container. Alpine's musl + `*-static` packages yield a single binary with
# no runtime library dependencies, which is what we need inside the initrd.
#
# The container image is pinned by sha256 digest — same reproducibility bar
# as the apt packages above. The cryptsetup source tarball is pinned by
# version + SHA256.
#
# Crypto backend: OpenSSL (openssl-libs-static). LUKS2 argon2id KDF uses
# libargon2 (also static). Optional features (asciidoc docs, ssh token
# plugin, external tokens, i18n) are disabled — not needed for our narrow
# open/format/close use in the initrd.
#
# The output binary ends up at $WORK_DIR/cryptsetup/cryptsetup-static and is
# copied into the initrd by the "Install Static cryptsetup" subsection in
# the Initrd Structure step below.

log_section "Build Static cryptsetup"

: "${CRYPTSETUP_VERSION:?CRYPTSETUP_VERSION not set - required for reproducible builds}"
: "${CRYPTSETUP_SHA256:?CRYPTSETUP_SHA256 not set - required for reproducible builds}"
: "${CRYPTSETUP_BUILDER_IMAGE:?CRYPTSETUP_BUILDER_IMAGE not set - pinned container image digest required}"

CRYPTSETUP_DIR="$WORK_DIR/cryptsetup"
mkdir -p "$CRYPTSETUP_DIR"
pushd "$CRYPTSETUP_DIR" >/dev/null

# kernel.org tarball URLs are organised under major.minor (e.g. v2.7).
CRYPTSETUP_MAJOR_MINOR="$(printf '%s' "$CRYPTSETUP_VERSION" | awk -F. '{print $1"."$2}')"
CRYPTSETUP_URL="https://www.kernel.org/pub/linux/utils/cryptsetup/v${CRYPTSETUP_MAJOR_MINOR}/cryptsetup-${CRYPTSETUP_VERSION}.tar.xz"
CRYPTSETUP_TARBALL="cryptsetup-${CRYPTSETUP_VERSION}.tar.xz"

log_info "Downloading $CRYPTSETUP_URL"
curl -fLsS -o "$CRYPTSETUP_TARBALL" "$CRYPTSETUP_URL"

log_info "Verifying cryptsetup source checksum"
ACTUAL_SHA256="$(sha256sum "$CRYPTSETUP_TARBALL" | awk '{print $1}')"
if [[ "$ACTUAL_SHA256" != "$CRYPTSETUP_SHA256" ]]; then
    die "cryptsetup source checksum mismatch (expected $CRYPTSETUP_SHA256, got $ACTUAL_SHA256)"
fi
log_ok "cryptsetup source checksum verified"

log_info "Extracting source"
tar -xf "$CRYPTSETUP_TARBALL"

log_info "Building statically inside $CRYPTSETUP_BUILDER_IMAGE"
# The container runs as root (apk add requires it). Once the build is done,
# chown the output binary to the invoking host user so subsequent host-side
# steps — including the trap's rm -rf "$WORK_DIR" — don't trip over root-
# owned files. SOURCE_DATE_EPOCH is forwarded so any timestamps embedded in
# the binary match the host's reproducibility anchor.
HOST_UID="$(id -u)"
HOST_GID="$(id -g)"
"$CRYPTSETUP_BUILDER" run --rm \
    -v "$CRYPTSETUP_DIR:/build" \
    -w "/build/cryptsetup-${CRYPTSETUP_VERSION}" \
    -e "SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}" \
    -e "HOST_UID=${HOST_UID}" \
    -e "HOST_GID=${HOST_GID}" \
    "$CRYPTSETUP_BUILDER_IMAGE" \
    sh -euc '
        apk add --no-cache \
            build-base linux-headers pkgconf \
            openssl-dev openssl-libs-static \
            popt-dev popt-static \
            json-c-dev \
            util-linux-dev util-linux-static \
            lvm2-dev lvm2-static \
            argon2-dev argon2-static \
            libblkid-static libuuid-static
        ./configure \
            --disable-shared \
            --enable-static \
            --with-crypto_backend=openssl \
            --disable-asciidoc \
            --disable-ssh-token \
            --disable-external-tokens \
            --disable-nls
        make -j"$(nproc)" LDFLAGS="-all-static"
        strip src/cryptsetup
        cp src/cryptsetup /build/cryptsetup-static
        chown "${HOST_UID}:${HOST_GID}" /build/cryptsetup-static
        # Intermediate build artefacts stay root-owned inside /build. The host
        # owns $CRYPTSETUP_DIR itself, so the trap'"'"'s rm -rf can still unlink
        # them; but make the leaf directories writable by the host user so any
        # follow-up inspection (find, ls) does not hit permission errors.
        chown -R "${HOST_UID}:${HOST_GID}" /build
    '

if [[ ! -x "$CRYPTSETUP_DIR/cryptsetup-static" ]]; then
    die "cryptsetup static build did not produce a binary at $CRYPTSETUP_DIR/cryptsetup-static"
fi

log_info "Verifying cryptsetup is statically linked"
LDD_OUT="$(ldd "$CRYPTSETUP_DIR/cryptsetup-static" 2>&1 || true)"
if echo "$LDD_OUT" | grep -qE "not a dynamic executable|statically linked"; then
    log_ok "cryptsetup is statically linked"
else
    log_warn "cryptsetup may not be fully static:"
    echo "$LDD_OUT" | sed 's/^/    /'
    die "cryptsetup must be statically linked to run in the initrd"
fi

log_info "Normalising timestamp for reproducibility"
touch -d "@${SOURCE_DATE_EPOCH}" "$CRYPTSETUP_DIR/cryptsetup-static"

popd >/dev/null

# ==============================================================================
# SECTION 4: Build Initrd Structure
# ==============================================================================

log_section "Build Initrd Structure"
INITRD_DIR="$WORK_DIR/initrd"
mkdir -p "$INITRD_DIR"/{bin,dev,proc,sys,tmp,etc,lib/modules,mnt}

cd "$INITRD_DIR"

# ------------------------------------------------------------------------------
# Install Busybox
# ------------------------------------------------------------------------------
log_info "Installing busybox"
BUSYBOX_BIN="$EXTRACTED_DIR/usr/bin/busybox"
[[ -f "$BUSYBOX_BIN" ]] || die "busybox binary not found in extracted package: $BUSYBOX_BIN"

cp "$BUSYBOX_BIN" bin/busybox
chmod +x bin/busybox

if ! bin/busybox --help >/dev/null 2>&1; then
    die "Copied busybox binary is not functional"
fi

AVAILABLE_APPLETS="$(bin/busybox --list 2>/dev/null || true)"
[[ -n "$AVAILABLE_APPLETS" ]] || die "Could not read busybox applet list"

MISSING_APPLETS=()
for applet in "${REQUIRED_APPLETS[@]}"; do
    if ! echo "$AVAILABLE_APPLETS" | grep -Fqx "$applet"; then
        MISSING_APPLETS+=("$applet")
    fi
done

if [[ ${#MISSING_APPLETS[@]} -gt 0 ]]; then
    die "busybox is missing required applets: ${MISSING_APPLETS[*]}"
fi

for cmd in "${SYMLINK_APPLETS[@]}"; do
    if echo "$AVAILABLE_APPLETS" | grep -Fqx "$cmd"; then
        ln -sf busybox "bin/$cmd"
    fi
done
log_ok "Busybox installed and applets validated"

# ------------------------------------------------------------------------------
# Install Static cryptsetup
# ------------------------------------------------------------------------------
# Built in SECTION 3 via a pinned Alpine container. Binary is fully static,
# so no .so files need to be vendored alongside it.
log_info "Installing static cryptsetup"
[[ -x "$CRYPTSETUP_DIR/cryptsetup-static" ]] \
    || die "cryptsetup-static not found at $CRYPTSETUP_DIR/cryptsetup-static (SECTION 3 did not run?)"

cp "$CRYPTSETUP_DIR/cryptsetup-static" bin/cryptsetup
chmod +x bin/cryptsetup

if ! bin/cryptsetup --version >/dev/null 2>&1; then
    die "Installed cryptsetup binary is not functional"
fi
log_ok "cryptsetup installed"

# ------------------------------------------------------------------------------
# Install SEV-SNP Kernel Modules
# ------------------------------------------------------------------------------
log_info "Installing SEV-SNP kernel modules"
MODULES_DIR="$EXTRACTED_DIR/lib/modules/$KERNEL_VERSION-generic/kernel/drivers/virt/coco"

if [[ -d "$MODULES_DIR" ]]; then
    if [[ -f "$MODULES_DIR/tsm.ko.zst" ]]; then
        zstd -dq "$MODULES_DIR/tsm.ko.zst" -o lib/modules/tsm.ko
        log_ok "tsm.ko installed (decompressed)"
    elif [[ -f "$MODULES_DIR/tsm.ko" ]]; then
        cp "$MODULES_DIR/tsm.ko" lib/modules/tsm.ko
        log_ok "tsm.ko installed"
    else
        log_warn "tsm.ko not found"
    fi

    if [[ -f "$MODULES_DIR/sev-guest/sev-guest.ko.zst" ]]; then
        zstd -dq "$MODULES_DIR/sev-guest/sev-guest.ko.zst" -o lib/modules/sev-guest.ko
        log_ok "sev-guest.ko installed (decompressed)"
    elif [[ -f "$MODULES_DIR/sev-guest/sev-guest.ko" ]]; then
        cp "$MODULES_DIR/sev-guest/sev-guest.ko" lib/modules/sev-guest.ko
        log_ok "sev-guest.ko installed"
    else
        log_warn "sev-guest.ko not found"
    fi
else
    log_warn "Modules directory not found: $MODULES_DIR"
    log_warn "SEV-SNP attestation may not be available"
fi

# ------------------------------------------------------------------------------
# Install Device-Mapper Kernel Modules
# ------------------------------------------------------------------------------
# Required by cryptsetup for LUKS2 open/format. Load order at runtime is
# dm-mod first, then dm-crypt and dm-integrity (both depend on dm-mod).
log_info "Installing device-mapper kernel modules"
DM_MODULES_DIR="$EXTRACTED_DIR/lib/modules/$KERNEL_VERSION-generic/kernel/drivers/md"

if [[ -d "$DM_MODULES_DIR" ]]; then
    for mod in dm-mod dm-crypt dm-integrity; do
        if [[ -f "$DM_MODULES_DIR/${mod}.ko.zst" ]]; then
            zstd -dq "$DM_MODULES_DIR/${mod}.ko.zst" -o "lib/modules/${mod}.ko"
            log_ok "${mod}.ko installed (decompressed)"
        elif [[ -f "$DM_MODULES_DIR/${mod}.ko" ]]; then
            cp "$DM_MODULES_DIR/${mod}.ko" "lib/modules/${mod}.ko"
            log_ok "${mod}.ko installed"
        else
            log_warn "${mod}.ko not found at $DM_MODULES_DIR/${mod}.ko(.zst)"
        fi
    done
else
    log_warn "Device-mapper modules directory not found: $DM_MODULES_DIR"
    log_warn "Sealed storage will not be available"
fi

# ------------------------------------------------------------------------------
# Install Katana Binary
# ------------------------------------------------------------------------------
log_info "Installing Katana binary"
cp "$KATANA_BINARY" bin/katana
chmod +x bin/katana
log_ok "Katana installed"

# ------------------------------------------------------------------------------
# Create Init Script
# ------------------------------------------------------------------------------
log_info "Creating init script"
cat > init <<'INIT_EOF'
#!/bin/sh
# Katana TEE VM Init Script

set -eu
export PATH=/bin

# log writes to stderr so command substitution like `$(strip_db_args ...)`
# captures only the function's real output. Both stdout and stderr are
# redirected to /dev/console below, so operator UX is unchanged.
log() { echo "[init] $*" >&2; }

KATANA_PID=""
KATANA_DB_DIR="/mnt/data/katana-db"
SHUTTING_DOWN=0
KATANA_EXIT_CODE="never"
CONTROL_PORT_NAME="org.katana.control.0"
CONTROL_PORT_LINK="/dev/virtio-ports/org.katana.control.0"

# Sealed-storage state (populated from /proc/cmdline; see parse_cmdline_vars).
# SEALED_MODE=1 means an encrypted /dev/sda backs /mnt/data and the derived key
# must match; SEALED_MODE=0 keeps the legacy plain-ext4 behaviour.
EXPECTED_LUKS_UUID=""
SEALED_MODE=0
LUKS_MAPPER_NAME="katana-data"
LUKS_MAPPER_DEV="/dev/mapper/${LUKS_MAPPER_NAME}"
LUKS_DEVICE="/dev/sda"
LUKS_OPENED=0

fatal_boot() {
    log "ERROR: $*"
    teardown_and_halt
}

# Unified teardown path, safe to call from any phase.
# Each step is idempotent — tolerates being called before the mount / before
# the LUKS device is opened / before Katana has started. No step is allowed to
# block; every unmount / luksClose runs with timeouts or `|| true` so a stuck
# filesystem cannot prevent the VM from powering off.
teardown_and_halt() {
    if [ "$SHUTTING_DOWN" -eq 1 ]; then
        # Re-entry (e.g. fatal_boot called from within shutdown_handler).
        # Keep spinning; the first caller is already driving the teardown.
        while true; do sleep 1; done
    fi
    SHUTTING_DOWN=1

    log "Teardown: stopping katana (if running)..."
    if [ -n "${KATANA_PID:-}" ] && kill -0 "$KATANA_PID" 2>/dev/null; then
        kill -TERM "$KATANA_PID" 2>/dev/null || true
        TIMEOUT=30
        while [ "$TIMEOUT" -gt 0 ] && kill -0 "$KATANA_PID" 2>/dev/null; do
            sleep 1
            TIMEOUT=$((TIMEOUT - 1))
        done
        if kill -0 "$KATANA_PID" 2>/dev/null; then
            log "Teardown: forcing kill of katana"
            kill -KILL "$KATANA_PID" 2>/dev/null || true
        fi
    fi

    log "Teardown: syncing and unmounting..."
    sync || true
    umount /mnt/data 2>/dev/null || true

    if [ "$LUKS_OPENED" -eq 1 ]; then
        log "Teardown: closing LUKS mapper $LUKS_MAPPER_NAME"
        /bin/cryptsetup luksClose "$LUKS_MAPPER_NAME" 2>/dev/null || true
        LUKS_OPENED=0
    fi

    umount /tmp 2>/dev/null || true
    umount /dev 2>/dev/null || true
    umount /sys/kernel/config 2>/dev/null || true
    umount /sys 2>/dev/null || true
    umount /proc 2>/dev/null || true

    log "Teardown: poweroff"
    poweroff -f
    while true; do sleep 1; done
}

# Parse KATANA_EXPECTED_LUKS_UUID out of /proc/cmdline. It is produced by
# start-vm.sh and lives inside the measured kernel command line — any tamper
# changes the launch measurement.
parse_cmdline_vars() {
    if [ ! -r /proc/cmdline ]; then
        log "WARNING: /proc/cmdline not readable; sealed-storage vars unset"
        return 0
    fi
    # Space-separated `key=value` tokens; iterate and pick out the one we care
    # about. We deliberately avoid `grep -oP` (GNU-only) to stay portable
    # against busybox variants.
    for tok in $(cat /proc/cmdline); do
        case "$tok" in
            KATANA_EXPECTED_LUKS_UUID=*)
                EXPECTED_LUKS_UUID="${tok#KATANA_EXPECTED_LUKS_UUID=}"
                ;;
        esac
    done
    if [ -n "$EXPECTED_LUKS_UUID" ]; then
        SEALED_MODE=1
        log "Sealed mode enabled: EXPECTED_LUKS_UUID=$EXPECTED_LUKS_UUID"
    else
        log "Sealed mode NOT enabled (KATANA_EXPECTED_LUKS_UUID unset)"
    fi
}

# Load device-mapper modules in dependency order. dm_mod must be loaded first
# because dm_crypt and dm_integrity register themselves against it.
load_dm_modules() {
    for mod in dm-mod dm-crypt dm-integrity; do
        if [ -f "/lib/modules/${mod}.ko" ]; then
            /bin/insmod "/lib/modules/${mod}.ko" 2>/dev/null \
                && log "Loaded ${mod}.ko" \
                || log "WARNING: insmod ${mod}.ko failed (may be builtin)"
        else
            log "WARNING: /lib/modules/${mod}.ko missing"
        fi
    done
}

# Drop any --db-dir / --db-* flags from a space-separated arg string. This
# prevents an operator with access to the virtio-serial control channel from
# pointing Katana at a db-dir outside the sealed mount, escaping the sealing
# guarantee. Runs inside the measured initrd, so the defense itself is pinned.
#
# Handles:  `--db-dir value`     (two tokens — next token consumed)
#           `--db-dir=value`     (one token)
#           `--db-anything[=val]` (catch-all for future --db-* flags)
strip_db_args() {
    SKIP_NEXT=0
    OUT=""
    for tok in $*; do
        if [ "$SKIP_NEXT" -eq 1 ]; then
            SKIP_NEXT=0
            log "strip_db_args: dropped value token '$tok'"
            continue
        fi
        case "$tok" in
            --db-*=*)
                log "strip_db_args: dropped '$tok'"
                continue
                ;;
            --db-*)
                log "strip_db_args: dropped '$tok' (and next token)"
                SKIP_NEXT=1
                continue
                ;;
        esac
        OUT="${OUT}${OUT:+ }${tok}"
    done
    echo "$OUT"
}

# Sealed-storage unlock. Called only when SEALED_MODE=1.
#
# Flow:
#   1. require /dev/sev-guest (else fatal)
#   2. if /dev/sda has no LUKS header, luksFormat with the expected UUID
#      (first-boot / post-wipe auto-provisioning — same measurement as
#      subsequent boots, so the derived key matches)
#   3. require the header's UUID to match the expected UUID
#      (else fatal: different disk mounted)
#   4. luksOpen (else fatal: chip or measurement drift — derived key
#      differs from what sealed the header)
#   5. if the decrypted mapper has no filesystem, mkfs.ext2
#   6. mount /dev/mapper/<name> at /mnt/data
#
# On first boot or after a hostile header-wipe, steps 2 and 5 both fire and
# the disk is recreated from scratch. An attacker who wipes the header does
# not gain state-substitution (they cannot produce blocks the verifier
# accepts unless the verifier independently pins the chain anchor); they
# only downgrade the chain to a fresh start. See docs/amdsev.md trust model.
#
# Note: busybox ships mkfs.ext2 but not mkfs.ext4. MDBX is indifferent; we
# accept ext2 for the sealed mount. Upgrading to a statically-built mke2fs
# later is an isolated follow-up.
KEY_FIFO="/tmp/katana-luks.key"

_unseal_spawn_key_writer() {
    rm -f "$KEY_FIFO"
    mkfifo -m 0600 "$KEY_FIFO" || teardown_and_halt "unseal: mkfifo failed"
    /bin/snp-derivekey > "$KEY_FIFO" &
    KEY_PID=$!
}

_unseal_wait_key_writer() {
    wait "$KEY_PID" 2>/dev/null || true
    KEY_PID=""
    rm -f "$KEY_FIFO"
}

unseal_and_mount() {
    [ -c /dev/sev-guest ] || teardown_and_halt "sealed mode requires /dev/sev-guest"
    [ -b "$LUKS_DEVICE" ] || teardown_and_halt "sealed mode: $LUKS_DEVICE not found"

    HAS_LUKS_HEADER=0
    if /bin/cryptsetup isLuks "$LUKS_DEVICE" 2>/dev/null; then
        HAS_LUKS_HEADER=1
    fi

    # First boot / post-wipe: format the blank disk with the expected UUID
    # under the current measurement's derived key. Subsequent boots with the
    # same measurement re-derive the same key and open cleanly.
    if [ "$HAS_LUKS_HEADER" -eq 0 ]; then
        log "No LUKS header on $LUKS_DEVICE; formatting with UUID=$EXPECTED_LUKS_UUID"
        _unseal_spawn_key_writer
        if ! /bin/cryptsetup --batch-mode \
                --type luks2 \
                --cipher aes-xts-plain64 --key-size 512 \
                --hash sha256 \
                --uuid "$EXPECTED_LUKS_UUID" \
                --integrity hmac-sha256 \
                --pbkdf pbkdf2 --pbkdf-force-iterations 1000 \
                --key-file "$KEY_FIFO" \
                luksFormat "$LUKS_DEVICE"; then
            _unseal_wait_key_writer
            teardown_and_halt "luksFormat failed"
        fi
        _unseal_wait_key_writer
    fi

    # Enforce header UUID matches the measured expectation.
    DISK_UUID="$(/bin/cryptsetup luksUUID "$LUKS_DEVICE" 2>/dev/null || true)"
    if [ "$DISK_UUID" != "$EXPECTED_LUKS_UUID" ]; then
        teardown_and_halt "sealed mode: disk UUID '$DISK_UUID' does not match expected '$EXPECTED_LUKS_UUID' (disk swapped?)"
    fi

    # Open. A failure here almost always means the derived key differs from
    # what the disk was sealed with — i.e. different chip, or the measured
    # image changed between sealing and now.
    _unseal_spawn_key_writer
    if ! /bin/cryptsetup --key-file "$KEY_FIFO" luksOpen "$LUKS_DEVICE" "$LUKS_MAPPER_NAME"; then
        _unseal_wait_key_writer
        teardown_and_halt "luksOpen failed — chip mismatch or measurement drift"
    fi
    _unseal_wait_key_writer
    LUKS_OPENED=1

    # Filesystem on the decrypted mapper. A missing filesystem only happens
    # on a fresh luksFormat (step 2 above) or if a prior boot failed between
    # format and mkfs — both are fine to (re-)format.
    if ! /bin/blkid "$LUKS_MAPPER_DEV" >/dev/null 2>&1; then
        log "Creating ext2 filesystem on $LUKS_MAPPER_DEV"
        /bin/mkfs.ext2 "$LUKS_MAPPER_DEV" >/dev/null 2>&1 \
            || teardown_and_halt "mkfs on decrypted volume failed"
    fi

    /bin/mount -t ext2 "$LUKS_MAPPER_DEV" /mnt/data \
        || teardown_and_halt "failed to mount $LUKS_MAPPER_DEV"
    log "Sealed storage mounted at /mnt/data"
}

refresh_katana_state() {
    if [ -n "$KATANA_PID" ] && ! kill -0 "$KATANA_PID" 2>/dev/null; then
        if wait "$KATANA_PID"; then
            KATANA_EXIT_CODE=0
        else
            KATANA_EXIT_CODE=$?
        fi
        log "Katana exited with code $KATANA_EXIT_CODE"
        KATANA_PID=""
    fi
}

respond_control() {
    printf '%s\n' "$1" >&3 2>/dev/null || true
}

resolve_control_port() {
    mkdir -p /dev/virtio-ports
    for name_file in /sys/class/virtio-ports/*/name; do
        [ -f "$name_file" ] || continue

        PORT_NAME_VALUE="$(cat "$name_file" 2>/dev/null || true)"
        if [ "$PORT_NAME_VALUE" != "$CONTROL_PORT_NAME" ]; then
            continue
        fi

        PORT_DIR="${name_file%/name}"
        PORT_DEV="/dev/${PORT_DIR##*/}"
        if [ -e "$PORT_DEV" ]; then
            ln -sf "$PORT_DEV" "$CONTROL_PORT_LINK"
            echo "$CONTROL_PORT_LINK"
            return 0
        fi
    done
    return 1
}

handle_control_command() {
    RAW_CMD="$1"
    CMD="${RAW_CMD%% *}"
    CMD_PAYLOAD=""
    if [ "$CMD" != "$RAW_CMD" ]; then
        CMD_PAYLOAD="${RAW_CMD#* }"
    fi

    case "$CMD" in
        start)
            refresh_katana_state
            if [ -n "$KATANA_PID" ] && kill -0 "$KATANA_PID" 2>/dev/null; then
                respond_control "err already-running pid=$KATANA_PID"
                return 0
            fi

            KATANA_ARGS=""
            if [ -n "$CMD_PAYLOAD" ]; then
                RAW_ARGS="$(echo "$CMD_PAYLOAD" | tr ',' ' ')"
                # Defense in depth: a later clap flag on the command line
                # would override the --db-dir we bake in below, letting an
                # operator point Katana at a directory outside the sealed
                # mount. Strip any --db-* from attacker-influenced input.
                KATANA_ARGS="$(strip_db_args $RAW_ARGS)"
            fi

            log "Starting katana asynchronously..."
            # shellcheck disable=SC2086
            /bin/katana --db-dir="$KATANA_DB_DIR" $KATANA_ARGS &
            KATANA_PID=$!
            KATANA_EXIT_CODE="running"
            respond_control "ok started pid=$KATANA_PID"
            ;;

        status)
            refresh_katana_state
            if [ -n "$KATANA_PID" ] && kill -0 "$KATANA_PID" 2>/dev/null; then
                respond_control "running pid=$KATANA_PID"
            else
                respond_control "stopped exit=$KATANA_EXIT_CODE"
            fi
            ;;

        "")
            ;;

        *)
            respond_control "err unknown-command"
            ;;
    esac
}

shutdown_handler() {
    log "Received shutdown signal"
    teardown_and_halt
}

trap shutdown_handler TERM INT

# Mount essential filesystems
/bin/mount -t proc proc /proc || log "WARNING: failed to mount /proc"
/bin/mount -t sysfs sysfs /sys || log "WARNING: failed to mount /sys"

# Mount /dev
if ! /bin/mount -t devtmpfs devtmpfs /dev 2>/dev/null; then
    /bin/mount -t tmpfs tmpfs /dev || log "WARNING: failed to mount /dev"
fi
/bin/mount -t tmpfs tmpfs /tmp 2>/dev/null || true

# Create essential device nodes
[ -c /dev/null ] || /bin/mknod /dev/null c 1 3 || true
[ -c /dev/console ] || /bin/mknod /dev/console c 5 1 || true
[ -c /dev/tty ] || /bin/mknod /dev/tty c 5 0 || true
[ -c /dev/urandom ] || /bin/mknod /dev/urandom c 1 9 || true

# Route all logs to the serial console
exec 0</dev/console
exec 1>/dev/console
exec 2>/dev/console

# Load SEV-SNP kernel modules
log "Loading SEV-SNP kernel modules..."
[ -f /lib/modules/tsm.ko ] && /bin/insmod /lib/modules/tsm.ko && log "Loaded tsm.ko" || true
[ -f /lib/modules/sev-guest.ko ] && /bin/insmod /lib/modules/sev-guest.ko && log "Loaded sev-guest.ko" || true
sleep 1

# Check for TEE attestation interfaces
TEE_DEVICE_FOUND=0

mkdir -p /sys/kernel/config
if /bin/mount -t configfs configfs /sys/kernel/config 2>/dev/null; then
    [ -d /sys/kernel/config/tsm/report ] && TEE_DEVICE_FOUND=1 && log "ConfigFS TSM interface available"
fi

if [ -c /dev/sev-guest ]; then
    TEE_DEVICE_FOUND=1
    log "SEV-SNP device available at /dev/sev-guest"
elif [ -f /sys/devices/virtual/misc/sev-guest/dev ]; then
    SEV_DEV="$(cat /sys/devices/virtual/misc/sev-guest/dev)"
    /bin/mknod /dev/sev-guest c "${SEV_DEV%%:*}" "${SEV_DEV##*:}" && TEE_DEVICE_FOUND=1 && log "Created /dev/sev-guest"
fi

for tpm in /dev/tpm0 /dev/tpmrm0; do
    [ -c "$tpm" ] && TEE_DEVICE_FOUND=1 && log "TPM device available at $tpm"
done

if [ "$TEE_DEVICE_FOUND" -eq 0 ]; then
    log "WARNING: No TEE attestation interface found"
fi

# Configure networking (QEMU user-mode defaults)
log "Configuring network..."
/bin/ip link set lo up 2>/dev/null || true
if [ -d /sys/class/net/eth0 ]; then
    /bin/ip link set eth0 up 2>/dev/null || true
    /bin/ip addr add 10.0.2.15/24 dev eth0 2>/dev/null || true
    /bin/ip route add default via 10.0.2.2 2>/dev/null || true
    echo "nameserver 10.0.2.3" > /etc/resolv.conf
    log "Network configured: eth0 = 10.0.2.15"
else
    log "WARNING: eth0 interface not found; skipping static network setup"
fi

# Parse sealed-storage vars out of the measured kernel cmdline.
parse_cmdline_vars

# Load dm-mod / dm-crypt / dm-integrity. Needed for sealed mode; harmless
# otherwise (modules just idle).
load_dm_modules

# Attach /dev/sda — either through LUKS (sealed) or directly (legacy).
if [ ! -b /dev/sda ]; then
    fatal_boot "required storage device /dev/sda not found"
fi
log "Found storage device /dev/sda"
mkdir -p /mnt/data

if [ "$SEALED_MODE" -eq 1 ]; then
    unseal_and_mount
else
    # Legacy path: plain ext4 on /dev/sda. Kept for backward compat with
    # non-sealed boot flows (CI, dev). Producing a quote under this path
    # is equivalent to the pre-H5 attestation model.
    if ! /bin/mount -t ext4 /dev/sda /mnt/data 2>/dev/null; then
        fatal_boot "failed to mount /dev/sda (unsealed)"
    fi
    log "Unsealed storage mounted at /mnt/data"
fi

mkdir -p "$KATANA_DB_DIR"

# Start async control loop for Katana startup/status commands.
log "Waiting for control channel ($CONTROL_PORT_NAME)..."
CONTROL_PORT=""
while [ -z "$CONTROL_PORT" ]; do
    CONTROL_PORT="$(resolve_control_port || true)"
    [ -n "$CONTROL_PORT" ] || sleep 1
done
log "Control channel ready: $CONTROL_PORT"

while true; do
    refresh_katana_state

    if ! exec 3<>"$CONTROL_PORT"; then
        log "WARNING: failed to open control channel, retrying..."
        sleep 1
        continue
    fi

    while IFS= read -r CONTROL_CMD <&3; do
        handle_control_command "$CONTROL_CMD"
    done

    exec 3>&- 3<&-
    sleep 1
done
INIT_EOF

chmod +x init
log_ok "Init script created"

# ------------------------------------------------------------------------------
# Create Minimal /etc Files
# ------------------------------------------------------------------------------
log_info "Creating /etc files"
echo "root:x:0:0:root:/:/bin/sh" > etc/passwd
echo "root:x:0:" > etc/group
log_ok "/etc files created"

# ==============================================================================
# SECTION 5: Create CPIO Archive
# ==============================================================================

log_section "Create CPIO Archive"
echo "Initrd contents:"
find . \( -type f -o -type l \) | sort
echo ""
echo "Total size before compression:"
du -sh .
echo ""

log_info "Setting timestamps to SOURCE_DATE_EPOCH (${SOURCE_DATE_EPOCH})"
find . -exec touch -h -d "@${SOURCE_DATE_EPOCH}" {} +

CPIO_FLAGS=(--create --format=newc --null --owner=0:0 --quiet)
if cpio --help 2>&1 | grep -q -- "--reproducible"; then
    CPIO_FLAGS+=(--reproducible)
else
    log_warn "cpio does not support --reproducible; continuing without it"
fi

log_info "Creating cpio archive"
find . -print0 | LC_ALL=C sort -z | cpio "${CPIO_FLAGS[@]}" | gzip -n > "$OUTPUT_INITRD"
touch -d "@${SOURCE_DATE_EPOCH}" "$OUTPUT_INITRD"

# ==============================================================================
# SECTION 6: Final Validation
# ==============================================================================

log_section "Final Validation"
[[ -f "$OUTPUT_INITRD" ]] || die "Output file was not created: $OUTPUT_INITRD"
[[ -s "$OUTPUT_INITRD" ]] || die "Output file is empty: $OUTPUT_INITRD"
if ! gzip -t "$OUTPUT_INITRD" 2>/dev/null; then
    die "Output file is not valid gzip: $OUTPUT_INITRD"
fi
log_ok "Output artifact validated"

echo ""
echo "=========================================="
echo "[OK] Initrd created successfully!"
echo "=========================================="
echo "Output file: $OUTPUT_INITRD"
echo "Size:        $(du -h "$OUTPUT_INITRD" | cut -f1)"
echo "SHA256:      $(sha256sum "$OUTPUT_INITRD" | cut -d' ' -f1)"
echo "=========================================="
