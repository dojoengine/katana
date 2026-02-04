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
#   - linux-modules-extra: Contains SEV-SNP kernel modules (tsm.ko, sev-guest.ko)
#
# Usage:
#   ./build-initrd.sh KATANA_BINARY OUTPUT_INITRD [KERNEL_VERSION]
#
# Environment:
#   SOURCE_DATE_EPOCH               REQUIRED. Unix timestamp for reproducible builds.
#   BUSYBOX_PKG_VERSION             REQUIRED. Exact apt package version (e.g., 1:1.36.1-6ubuntu3.1)
#   BUSYBOX_PKG_SHA256              REQUIRED. SHA256 checksum of the .deb package
#   KERNEL_MODULES_EXTRA_PKG_VERSION REQUIRED. Exact apt package version
#   KERNEL_MODULES_EXTRA_PKG_SHA256  REQUIRED. SHA256 checksum of the .deb package
#
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

function usage()
{
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
    echo "  KERNEL_MODULES_EXTRA_PKG_VERSION Exact apt package version for linux-modules-extra"
    echo "  KERNEL_MODULES_EXTRA_PKG_SHA256  SHA256 checksum of the linux-modules-extra .deb"
    echo ""
    echo "EXAMPLES:"
    echo "  export SOURCE_DATE_EPOCH=\$(date +%s)"
    echo "  export BUSYBOX_PKG_VERSION='1:1.36.1-6ubuntu3.1'"
    echo "  export BUSYBOX_PKG_SHA256='abc123...'"
    echo "  export KERNEL_MODULES_EXTRA_PKG_VERSION='6.8.0-90.99'"
    echo "  export KERNEL_MODULES_EXTRA_PKG_SHA256='def456...'"
    echo "  $0 ./katana ./initrd.img 6.8.0-90"
    exit 1
}

# Show help if requested or insufficient arguments
if [[ $# -lt 2 ]] || [[ "${1:-}" == "-h" ]] || [[ "${1:-}" == "--help" ]]; then
    usage
fi

KATANA_BINARY="$1"
OUTPUT_INITRD="$2"
# Use argument, or KERNEL_VERSION env var if set
KERNEL_VERSION="${3:-${KERNEL_VERSION:?KERNEL_VERSION must be set or passed as third argument}}"

echo "=========================================="
echo "Building Initrd"
echo "=========================================="
echo "Configuration:"
echo "  Katana binary:         $KATANA_BINARY"
echo "  Output initrd:         $OUTPUT_INITRD"
echo "  Kernel version:        $KERNEL_VERSION"
echo "  SOURCE_DATE_EPOCH:     ${SOURCE_DATE_EPOCH:-<not set>}"
echo ""
echo "Package versions:"
echo "  busybox-static:        ${BUSYBOX_PKG_VERSION:-<not set>}"
echo "  linux-modules-extra:   ${KERNEL_MODULES_EXTRA_PKG_VERSION:-<not set>}"
echo "=========================================="
echo ""

# Validate SOURCE_DATE_EPOCH
if [[ -z "${SOURCE_DATE_EPOCH:-}" ]]; then
    echo "ERROR: SOURCE_DATE_EPOCH must be set for reproducible builds"
    echo ""
    echo "Set SOURCE_DATE_EPOCH to a fixed timestamp, e.g.:"
    echo "  export SOURCE_DATE_EPOCH=\$(date +%s)"
    echo "  export SOURCE_DATE_EPOCH=\$(git log -1 --format=%ct)"
    exit 1
fi

# Validate Katana binary
if [[ ! -f "$KATANA_BINARY" ]]; then
    echo "ERROR: Katana binary not found: $KATANA_BINARY"
    exit 1
fi

# Create working directory
WORK_DIR=$(mktemp -d)

cleanup() {
    local exit_code=$?
    if [[ -d "$WORK_DIR" ]]; then
        rm -rf "$WORK_DIR"
    fi
    exit $exit_code
}

# Ensure cleanup on exit, interrupt, or termination
trap cleanup EXIT INT TERM

echo "Working directory: $WORK_DIR"
echo ""

# ==============================================================================
# SECTION 1: Download Required Packages
# ==============================================================================

echo "Downloading required packages..."
PACKAGES_DIR="$WORK_DIR/packages"
mkdir -p "$PACKAGES_DIR"

pushd "$PACKAGES_DIR" >/dev/null

# Require version pinning for reproducibility
: "${BUSYBOX_PKG_VERSION:?BUSYBOX_PKG_VERSION not set - required for reproducible builds}"
: "${KERNEL_MODULES_EXTRA_PKG_VERSION:?KERNEL_MODULES_EXTRA_PKG_VERSION not set - required for reproducible builds}"

# Download busybox-static (pinned version)
echo "  Downloading busybox-static=${BUSYBOX_PKG_VERSION}..."
apt-get download "busybox-static=${BUSYBOX_PKG_VERSION}" 2>&1 | grep -v "^W:"

# Download kernel modules package (pinned version)
echo "  Downloading linux-modules-extra-${KERNEL_VERSION}-generic=${KERNEL_MODULES_EXTRA_PKG_VERSION}..."
apt-get download "linux-modules-extra-${KERNEL_VERSION}-generic=${KERNEL_MODULES_EXTRA_PKG_VERSION}" 2>&1 | grep -v "^W:"

echo ""
echo "Downloaded packages:"
ls -lh *.deb

# Require checksum verification for reproducibility
: "${BUSYBOX_PKG_SHA256:?BUSYBOX_PKG_SHA256 not set - required for reproducible builds}"

echo ""
echo "Verifying busybox-static checksum..."
ACTUAL_SHA256=$(sha256sum busybox-static_*.deb | awk '{print $1}')
if [[ "$ACTUAL_SHA256" != "$BUSYBOX_PKG_SHA256" ]]; then
    echo "ERROR: busybox-static checksum mismatch!"
    echo "  Expected: $BUSYBOX_PKG_SHA256"
    echo "  Actual:   $ACTUAL_SHA256"
    exit 1
fi
echo "[OK] busybox-static checksum verified"

: "${KERNEL_MODULES_EXTRA_PKG_SHA256:?KERNEL_MODULES_EXTRA_PKG_SHA256 not set - required for reproducible builds}"

echo "Verifying linux-modules-extra checksum..."
ACTUAL_SHA256=$(sha256sum linux-modules-extra-*.deb | awk '{print $1}')
if [[ "$ACTUAL_SHA256" != "$KERNEL_MODULES_EXTRA_PKG_SHA256" ]]; then
    echo "ERROR: linux-modules-extra checksum mismatch!"
    echo "  Expected: $KERNEL_MODULES_EXTRA_PKG_SHA256"
    echo "  Actual:   $ACTUAL_SHA256"
    exit 1
fi
echo "[OK] linux-modules-extra checksum verified"
echo ""

popd >/dev/null

# ==============================================================================
# SECTION 2: Extract Packages
# ==============================================================================

echo "Extracting packages..."
EXTRACTED_DIR="$WORK_DIR/extracted"
mkdir -p "$EXTRACTED_DIR"

# Extract busybox
echo "  Extracting busybox-static..."
dpkg-deb -x "$PACKAGES_DIR"/busybox-static_*.deb "$EXTRACTED_DIR"

# Extract kernel modules
echo "  Extracting linux-modules-extra..."
dpkg-deb -x "$PACKAGES_DIR"/linux-modules-extra-*.deb "$EXTRACTED_DIR"

echo "[OK] Packages extracted"
echo ""

# ==============================================================================
# SECTION 3: Build Initrd Structure
# ==============================================================================

echo "Creating initrd directory structure..."
INITRD_DIR="$WORK_DIR/initrd"
mkdir -p "$INITRD_DIR"/{bin,dev,proc,sys,tmp,etc,lib/modules}

cd "$INITRD_DIR"

# ------------------------------------------------------------------------------
# Install Busybox
# ------------------------------------------------------------------------------
echo "Installing busybox..."
cp "$EXTRACTED_DIR/usr/bin/busybox" bin/busybox
chmod +x bin/busybox

# Create symlinks for required applets
for cmd in sh mount umount mkdir mknod switch_root ip insmod; do
    ln -s busybox "bin/$cmd"
done
echo "[OK] Busybox installed with symlinks"

# ------------------------------------------------------------------------------
# Install SEV-SNP Kernel Modules
# ------------------------------------------------------------------------------
echo "Installing SEV-SNP kernel modules..."
MODULES_DIR="$EXTRACTED_DIR/lib/modules/$KERNEL_VERSION-generic/kernel/drivers/virt/coco"

if [[ -d "$MODULES_DIR" ]]; then
    # Copy and decompress tsm.ko
    if [[ -f "$MODULES_DIR/tsm.ko.zst" ]]; then
        zstd -dq "$MODULES_DIR/tsm.ko.zst" -o lib/modules/tsm.ko
        echo "  [OK] tsm.ko (decompressed)"
    elif [[ -f "$MODULES_DIR/tsm.ko" ]]; then
        cp "$MODULES_DIR/tsm.ko" lib/modules/tsm.ko
        echo "  [OK] tsm.ko"
    else
        echo "  [WARN] tsm.ko not found"
    fi

    # Copy and decompress sev-guest.ko
    if [[ -f "$MODULES_DIR/sev-guest/sev-guest.ko.zst" ]]; then
        zstd -dq "$MODULES_DIR/sev-guest/sev-guest.ko.zst" -o lib/modules/sev-guest.ko
        echo "  [OK] sev-guest.ko (decompressed)"
    elif [[ -f "$MODULES_DIR/sev-guest/sev-guest.ko" ]]; then
        cp "$MODULES_DIR/sev-guest/sev-guest.ko" lib/modules/sev-guest.ko
        echo "  [OK] sev-guest.ko"
    else
        echo "  [WARN] sev-guest.ko not found"
    fi
else
    echo "  [WARN] Modules directory not found: $MODULES_DIR"
    echo "  SEV-SNP attestation may not be available"
fi

# ------------------------------------------------------------------------------
# Install Katana Binary
# ------------------------------------------------------------------------------
echo "Installing Katana binary..."
cp "$KATANA_BINARY" bin/katana
chmod +x bin/katana
echo "[OK] Katana installed"

# ------------------------------------------------------------------------------
# Create Init Script
# ------------------------------------------------------------------------------
echo "Creating init script..."
cat > init <<'INIT_EOF'
#!/bin/busybox sh
# Katana TEE VM Init Script

set -eu
export PATH=/bin

log() { echo "[init] $*"; }

# Mount essential filesystems
/bin/mount -t proc proc /proc || log "WARNING: failed to mount /proc"
/bin/mount -t sysfs sysfs /sys || log "WARNING: failed to mount /sys"

# Mount /dev
if ! /bin/mount -t devtmpfs devtmpfs /dev 2>/dev/null; then
    /bin/mount -t tmpfs tmpfs /dev || log "WARNING: failed to mount /dev"
fi
/bin/mount -t tmpfs tmpfs /tmp 2>/dev/null || true

# Create essential device nodes
[ -c /dev/null ]    || /bin/mknod /dev/null c 1 3 || true
[ -c /dev/console ] || /bin/mknod /dev/console c 5 1 || true
[ -c /dev/tty ]     || /bin/mknod /dev/tty c 5 0 || true
[ -c /dev/urandom ] || /bin/mknod /dev/urandom c 1 9 || true

# Load SEV-SNP kernel modules
log "Loading SEV-SNP kernel modules..."
[ -f /lib/modules/tsm.ko ] && /bin/insmod /lib/modules/tsm.ko && log "Loaded tsm.ko" || true
[ -f /lib/modules/sev-guest.ko ] && /bin/insmod /lib/modules/sev-guest.ko && log "Loaded sev-guest.ko" || true
sleep 1

# Check for TEE attestation interfaces
TEE_DEVICE_FOUND=0

# Check ConfigFS TSM interface
if /bin/mount -t configfs configfs /sys/kernel/config 2>/dev/null; then
    [ -d /sys/kernel/config/tsm/report ] && TEE_DEVICE_FOUND=1 && log "ConfigFS TSM interface available"
fi

# Check SEV-SNP legacy device
if [ -c /dev/sev-guest ]; then
    TEE_DEVICE_FOUND=1
    log "SEV-SNP device available at /dev/sev-guest"
elif [ -f /sys/devices/virtual/misc/sev-guest/dev ]; then
    SEV_DEV=$(cat /sys/devices/virtual/misc/sev-guest/dev)
    /bin/mknod /dev/sev-guest c "${SEV_DEV%%:*}" "${SEV_DEV##*:}" && TEE_DEVICE_FOUND=1 && log "Created /dev/sev-guest"
fi

# Check TPM devices
for tpm in /dev/tpm0 /dev/tpmrm0; do
    [ -c "$tpm" ] && TEE_DEVICE_FOUND=1 && log "TPM device available at $tpm"
done

if [ "$TEE_DEVICE_FOUND" -eq 0 ]; then
    log "WARNING: No TEE attestation interface found"
fi

# Configure networking (QEMU user-mode defaults)
log "Configuring network..."
ip link set lo up 2>/dev/null || true
ip link set eth0 up 2>/dev/null || true
ip addr add 10.0.2.15/24 dev eth0 2>/dev/null || true
ip route add default via 10.0.2.2 2>/dev/null || true

# Configure DNS (QEMU user-mode provides DNS on 10.0.2.3)
mkdir -p /etc
echo "nameserver 10.0.2.3" > /etc/resolv.conf
log "DNS configured (10.0.2.3)"

# Configure SSL/TLS certificates for HTTPS
if [ -f /etc/ssl/certs/ca-certificates.crt ]; then
    export SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt
    log "CA certificates configured for HTTPS"
fi

# Parse katana args from cmdline
CMDLINE="$(cat /proc/cmdline 2>/dev/null || true)"
KATANA_ARGS=""
for tok in $CMDLINE; do
    case "$tok" in
        katana.args=*) KATANA_ARGS="$(echo "${tok#katana.args=}" | tr ',' ' ')" ;;
    esac
done

# Mount persistent storage
if [ -b /dev/sda ]; then
    log "Found storage device /dev/sda"
    mkdir -p /mnt/data
    if ! /bin/mount -t ext4 /dev/sda /mnt/data 2>/dev/null; then
        log "Formatting /dev/sda..."
        /sbin/mkfs.ext4 -F /dev/sda && /bin/mount -t ext4 /dev/sda /mnt/data
    fi
    mkdir -p /mnt/data/katana-db
    log "Storage mounted at /mnt/data"
    # shellcheck disable=SC2086
    exec /bin/katana --db-dir="/mnt/data/katana-db" $KATANA_ARGS 2>&1
else
    log "No storage device found, running without persistence"
    # shellcheck disable=SC2086
    exec /bin/katana $KATANA_ARGS 2>&1
fi
INIT_EOF

chmod +x init
echo "[OK] Init script created"

# ------------------------------------------------------------------------------
# Create Minimal /etc Files
# ------------------------------------------------------------------------------
echo "Creating /etc files..."
echo "root:x:0:0:root:/root:/bin/sh" > etc/passwd
echo "root:x:0:" > etc/group

# Copy CA certificates for HTTPS/TLS support
echo "Installing CA certificates..."
mkdir -p etc/ssl/certs
if [ -f /etc/ssl/certs/ca-certificates.crt ]; then
    cp /etc/ssl/certs/ca-certificates.crt etc/ssl/certs/ca-certificates.crt
    echo "[OK] CA certificates installed from /etc/ssl/certs/ca-certificates.crt"
elif [ -f /usr/share/ca-certificates/ca-certificates.crt ]; then
    cp /usr/share/ca-certificates/ca-certificates.crt etc/ssl/certs/ca-certificates.crt
    echo "[OK] CA certificates installed from /usr/share/ca-certificates/ca-certificates.crt"
else
    echo "[WARN] CA certificates not found on host - HTTPS connections may fail"
fi

echo "[OK] /etc files created"

# ==============================================================================
# SECTION 4: Create CPIO Archive
# ==============================================================================

echo ""
echo "Initrd contents:"
find . -type f -o -type l | sort
echo ""
echo "Total size before compression:"
du -sh .
echo ""

# Normalize timestamps for reproducibility
echo "Setting timestamps to SOURCE_DATE_EPOCH ($SOURCE_DATE_EPOCH)..."
find . -exec touch -h -d "@${SOURCE_DATE_EPOCH}" {} +

# Create compressed cpio archive
echo "Creating cpio archive..."
find . -print0 | LC_ALL=C sort -z | cpio --create --format=newc --null --owner=0:0 --quiet | gzip -n > "$OUTPUT_INITRD"

echo ""
echo "=========================================="
echo "[OK] Initrd created successfully!"
echo "=========================================="
echo "Output file: $OUTPUT_INITRD"
echo "Size:        $(du -h "$OUTPUT_INITRD" | cut -f1)"
echo "SHA256:      $(sha256sum "$OUTPUT_INITRD" | cut -d' ' -f1)"
echo "=========================================="
