#!/bin/bash
set -euo pipefail

KATANA_BINARY=$1
OUTPUT_INITRD=$2
KERNEL_VERSION="${3:-auto}"  # Auto-detect from extracted packages or use specified version

echo "=========================================="
echo "Creating Initrd (Debug Mode)"
echo "=========================================="
echo "Configuration:"
echo "  Katana binary:       $KATANA_BINARY"
echo "  Output initrd:       $OUTPUT_INITRD"
echo "  Kernel version:      $KERNEL_VERSION"
echo "  SOURCE_DATE_EPOCH:   $SOURCE_DATE_EPOCH"
echo "=========================================="
echo ""

# Verify SOURCE_DATE_EPOCH is set
if [[ -z "${SOURCE_DATE_EPOCH:-}" ]]; then
    echo "ERROR: SOURCE_DATE_EPOCH must be set"
    exit 1
fi

# Verify Katana binary exists
if [[ ! -f "$KATANA_BINARY" ]]; then
    echo "ERROR: Katana binary not found: $KATANA_BINARY"
    exit 1
fi

echo "Katana binary info:"
ls -lh "$KATANA_BINARY"
file "$KATANA_BINARY" || true
echo ""

# Create temporary directory for initrd contents
echo "Creating temporary directory for initrd contents..."
INITRD_DIR=$(mktemp -d)
trap "rm -rf $INITRD_DIR" EXIT
echo "✓ Working directory: $INITRD_DIR"
echo ""

cd "$INITRD_DIR"

# Create directory structure
echo "Creating initrd directory structure..."
mkdir -p bin dev proc sys tmp etc
echo "✓ Directories created"
echo ""

# Copy busybox (check multiple locations)
echo "Copying busybox..."
if [[ -f /components/busybox ]]; then
    cp /components/busybox bin/busybox
    echo "✓ Busybox copied from /components/busybox"
elif [[ -f /bin/busybox ]]; then
    cp /bin/busybox bin/busybox
    echo "✓ Busybox copied from /bin/busybox"
else
    echo "ERROR: busybox not found at /components/busybox or /bin/busybox"
    exit 1
fi
echo ""

# Create busybox symlinks
echo "Creating busybox symlinks..."
for cmd in sh mount umount mkdir mknod switch_root ip insmod; do
    ln -s busybox bin/$cmd
    echo "  - bin/$cmd -> busybox"
done
echo "✓ Symlinks created"
echo ""

# Copy SEV-SNP kernel modules
echo "Copying SEV-SNP kernel modules..."
mkdir -p lib/modules

# Auto-detect kernel version if needed (check Docker extracted path first, then host)
if [[ "$KERNEL_VERSION" == "auto" ]]; then
    if [[ -d /extracted/lib/modules ]]; then
        KERNEL_VERSION=$(ls /extracted/lib/modules/ 2>/dev/null | head -1)
        echo "  Auto-detected kernel version from /extracted: $KERNEL_VERSION"
    elif [[ -d /lib/modules ]]; then
        KERNEL_VERSION=$(ls /lib/modules/ 2>/dev/null | grep -v "$(uname -r)" | head -1)
        [[ -z "$KERNEL_VERSION" ]] && KERNEL_VERSION=$(uname -r)
        echo "  Auto-detected kernel version from /lib/modules: $KERNEL_VERSION"
    fi
fi

# Check multiple locations for modules (Docker extracted path first, then host path)
MODULES_FOUND=0
for BASE_PATH in "/extracted/lib/modules" "/lib/modules"; do
    MODULES_DIR="$BASE_PATH/$KERNEL_VERSION/kernel/drivers/virt/coco"
    if [[ -d "$MODULES_DIR" ]]; then
        echo "  Found modules directory at: $MODULES_DIR"

        # Copy tsm.ko
        if [[ -f "$MODULES_DIR/tsm.ko.zst" ]]; then
            zstd -d "$MODULES_DIR/tsm.ko.zst" -o lib/modules/tsm.ko
            echo "  ✓ tsm.ko"
            MODULES_FOUND=1
        elif [[ -f "$MODULES_DIR/tsm.ko" ]]; then
            cp "$MODULES_DIR/tsm.ko" lib/modules/tsm.ko
            echo "  ✓ tsm.ko (uncompressed)"
            MODULES_FOUND=1
        else
            echo "  WARNING: tsm.ko not found in $MODULES_DIR"
        fi

        # Copy sev-guest.ko
        if [[ -f "$MODULES_DIR/sev-guest/sev-guest.ko.zst" ]]; then
            zstd -d "$MODULES_DIR/sev-guest/sev-guest.ko.zst" -o lib/modules/sev-guest.ko
            echo "  ✓ sev-guest.ko"
            MODULES_FOUND=1
        elif [[ -f "$MODULES_DIR/sev-guest/sev-guest.ko" ]]; then
            cp "$MODULES_DIR/sev-guest/sev-guest.ko" lib/modules/sev-guest.ko
            echo "  ✓ sev-guest.ko (uncompressed)"
            MODULES_FOUND=1
        else
            echo "  WARNING: sev-guest.ko not found in $MODULES_DIR/sev-guest"
        fi

        break  # Found modules directory, stop searching
    fi
done

if [[ $MODULES_FOUND -eq 0 ]]; then
    echo "  WARNING: SEV-SNP kernel modules not found"
    echo "  Searched in: /extracted/lib/modules/$KERNEL_VERSION and /lib/modules/$KERNEL_VERSION"
    echo "  The VM will not have SEV-SNP attestation support"
else
    echo "  Modules copied to initramfs:"
    ls -la lib/modules/ 2>/dev/null || true
fi
echo "✓ Kernel modules step completed"
echo ""

# Copy Katana binary
echo "Copying Katana binary..."
cp "$KATANA_BINARY" bin/katana
chmod +x bin/katana
echo "✓ Katana copied to bin/katana"
ls -lh bin/katana
echo ""

# Create init script with debug output
cat > init <<'EOF'
#!/bin/busybox sh
set -eu
export PATH=/bin

log() { echo "[init] $*"; }
dbg() { if [ "${DEBUG:-0}" -eq 1 ]; then echo "[init][debug] $*"; fi; }

# Mount /proc first so we can read cmdline
/bin/mount -t proc proc /proc || log "WARNING: failed to mount /proc"

# Now read cmdline
CMDLINE="$(cat /proc/cmdline 2>/dev/null || true)"

DEBUG=0
for tok in $CMDLINE; do
  case "$tok" in
    katana.debug=1|debug=1) DEBUG=1 ;;
  esac
done

log "Katana TEE Init - starting (debug=$DEBUG)"

# Mount other virtual filesystems
/bin/mount -t sysfs sysfs /sys || log "WARNING: failed to mount /sys"

# /dev: prefer devtmpfs, fallback to tmpfs (rare, but safer)
if ! /bin/mount -t devtmpfs devtmpfs /dev 2>/dev/null; then
  log "devtmpfs mount failed; falling back to tmpfs"
  /bin/mount -t tmpfs tmpfs /dev || log "WARNING: failed to mount /dev"
fi

# /tmp hygiene
/bin/mount -t tmpfs tmpfs /tmp 2>/dev/null || true

# Essential device nodes
[ -c /dev/null ]    || /bin/mknod /dev/null c 1 3 || true
[ -c /dev/console ] || /bin/mknod /dev/console c 5 1 || true
[ -c /dev/tty ]     || /bin/mknod /dev/tty c 5 0 || true
[ -c /dev/urandom ] || /bin/mknod /dev/urandom c 1 9 || true

# --- Load SEV-SNP kernel modules ---
# The sev-guest driver is built as a module in Ubuntu kernels
log "Loading SEV-SNP kernel modules..."
if [ -f /lib/modules/tsm.ko ]; then
  log "Found tsm.ko, loading..."
  if /bin/insmod /lib/modules/tsm.ko; then
    log "Loaded tsm.ko"
  else
    log "Failed to load tsm.ko (exit code: $?)"
  fi
else
  log "tsm.ko not found in initramfs"
fi

if [ -f /lib/modules/sev-guest.ko ]; then
  log "Found sev-guest.ko, loading..."
  if /bin/insmod /lib/modules/sev-guest.ko; then
    log "Loaded sev-guest.ko"
  else
    log "Failed to load sev-guest.ko (exit code: $?)"
  fi
else
  log "sev-guest.ko not found in initramfs"
fi

# Give udev/kernel time to create device nodes
sleep 1

# --- TEE Attestation Devices ---
# Support multiple attestation backends:
# - configfs TSM: /sys/kernel/config/tsm/report (modern kernel interface)
# - SEV legacy: /dev/sev-guest (AMD SEV-SNP)
# - TPM: /dev/tpm0, /dev/tpmrm0

TEE_DEVICE_FOUND=0

# 1. ConfigFS TSM interface (modern kernels)
log "Checking for configfs TSM interface..."
if /bin/mount -t configfs configfs /sys/kernel/config 2>/dev/null; then
  if [ -d /sys/kernel/config/tsm/report ]; then
    log "ConfigFS TSM interface available at /sys/kernel/config/tsm/report"
    TEE_DEVICE_FOUND=1
  else
    dbg "ConfigFS mounted but TSM report interface not available"
  fi
else
  dbg "ConfigFS mount failed (may not be supported)"
fi

# 2. AMD SEV-SNP legacy device (/dev/sev-guest)
log "Checking for SEV-SNP legacy device..."
if [ -c /dev/sev-guest ]; then
  log "SEV-SNP attestation device available at /dev/sev-guest"
  TEE_DEVICE_FOUND=1
else
  # Try to create it manually (misc device, major 10, minor from /sys)
  if [ -f /sys/devices/virtual/misc/sev-guest/dev ]; then
    SEV_DEV=$(cat /sys/devices/virtual/misc/sev-guest/dev)
    SEV_MAJOR=${SEV_DEV%%:*}
    SEV_MINOR=${SEV_DEV##*:}
    if /bin/mknod /dev/sev-guest c "$SEV_MAJOR" "$SEV_MINOR"; then
      log "SEV-SNP attestation device created at /dev/sev-guest (major=$SEV_MAJOR, minor=$SEV_MINOR)"
      TEE_DEVICE_FOUND=1
    else
      log "WARNING: Failed to create /dev/sev-guest (major=$SEV_MAJOR, minor=$SEV_MINOR)"
    fi
  else
    dbg "SEV-SNP legacy device not available"
  fi
fi

# 3. TPM devices (/dev/tpm0, /dev/tpmrm0)
log "Checking for TPM devices..."
for tpm_path in /dev/tpm0 /dev/tpmrm0; do
  if [ -c "$tpm_path" ]; then
    log "TPM device available at $tpm_path"
    TEE_DEVICE_FOUND=1
  else
    # Try to create from sysfs
    tpm_name=$(basename "$tpm_path")
    sysfs_dev="/sys/class/tpm/${tpm_name}/dev"
    if [ -f "$sysfs_dev" ]; then
      TPM_DEV=$(cat "$sysfs_dev")
      TPM_MAJOR=${TPM_DEV%%:*}
      TPM_MINOR=${TPM_DEV##*:}
      if /bin/mknod "$tpm_path" c "$TPM_MAJOR" "$TPM_MINOR"; then
        log "TPM device created at $tpm_path (major=$TPM_MAJOR, minor=$TPM_MINOR)"
        TEE_DEVICE_FOUND=1
      else
        dbg "Failed to create $tpm_path"
      fi
    else
      dbg "TPM device $tpm_path not available"
    fi
  fi
done

# Require at least one TEE attestation interface
if [ "$TEE_DEVICE_FOUND" -eq 0 ]; then
  log "ERROR: No TEE attestation interface found"
  log "ERROR: Checked: configfs TSM, /dev/sev-guest, /dev/tpm0, /dev/tpmrm0"
  log "ERROR: This VM may not be running in a TEE environment"
  exec /bin/sh  # Drop to shell for debugging
fi

log "TEE attestation interface(s) initialized"

dbg "Mounted filesystems:"
[ "$DEBUG" -eq 1 ] && /bin/mount || true

dbg "Kernel cmdline: $CMDLINE"

# Configure networking
log "Configuring network interfaces..."
# Bring up loopback
ip link set lo up 2>/dev/null || log "WARNING: loopback setup failed"
# Bring up eth0 with DHCP (for QEMU user networking)
ip link set eth0 up 2>/dev/null || log "WARNING: eth0 setup failed"
# Simple static IP (QEMU user networking default)
ip addr add 10.0.2.15/24 dev eth0 2>/dev/null || log "WARNING: eth0 IP setup failed"
ip route add default via 10.0.2.2 2>/dev/null || log "WARNING: default route failed"
dbg "Network configured"

# Parse katana args from cmdline
KATANA_ARGS=""
for tok in $CMDLINE; do
  case "$tok" in
    katana.args=*)
      # Convert commas to spaces to allow multi-arg passing via cmdline
      KATANA_ARGS="$(echo "${tok#katana.args=}" | tr ',' ' ')"
      ;;
  esac
done

# Require persistent storage device (SCSI disk)
if [ ! -b /dev/sda ]; then
  log "ERROR: /dev/sda not found"
  log "ERROR: Add a disk to the VM"
  exec /bin/sh  # Drop to shell for debugging
fi

log "Found storage device /dev/sda"

# Create mount point
mkdir -p /mnt/data 2>/dev/null || true

# Mount or format the disk
if ! /bin/mount -t ext4 /dev/sda /mnt/data 2>/dev/null; then
    log "Mount failed, attempting to format /dev/sda..."

    if /sbin/mkfs.ext4 -F /dev/sda; then
        log "Format successful, mounting..."
        /bin/mount -t ext4 /dev/sda /mnt/data
    else
        log "ERROR: Could not format or mount /dev/sda"
        exec /bin/sh
    fi
fi

log "Storage mounted at /mnt/data"

# Ensure storage directory exists
mkdir -p "/mnt/data/katana-db" 2>/dev/null || true
dbg "Storage directory: /mnt/data/katana-db"
dbg "Katana args: $KATANA_ARGS"
log "Launching Katana with --db-dir=/mnt/data/katana-db..."

# shellcheck disable=SC2086
echo "test stderr" >&2
exec /bin/katana --db-dir="/mnt/data/katana-db" $KATANA_ARGS 2>&1
# exec /bin/katana --db-dir="/mnt/data/katana-db" $KATANA_ARGS
EOF

chmod +x init

# Create minimal /etc/passwd and /etc/group for compatibility
cat > etc/passwd <<EOF
root:x:0:0:root:/root:/bin/sh
EOF

cat > etc/group <<EOF
root:x:0:
EOF

# Show initrd contents before packing
echo "Initrd contents:"
find . -type f -o -type l | sort
echo ""
echo "Total size before compression:"
du -sh .
echo ""

# Set all timestamps to SOURCE_DATE_EPOCH for reproducibility
echo "Setting timestamps to SOURCE_DATE_EPOCH ($SOURCE_DATE_EPOCH)..."
find . -exec touch -h -d "@${SOURCE_DATE_EPOCH}" {} +
echo "✓ Timestamps normalized"
echo ""

# Create cpio archive with deterministic ordering
echo "Creating cpio archive and compressing..."
# Sort filenames for reproducibility
find . -print0 | LC_ALL=C sort -z | cpio --create --format=newc --null --owner=0:0 --quiet | gzip -n > "$OUTPUT_INITRD"
echo "✓ Initrd created"
echo ""

echo "=========================================="
echo "✓ Initrd created successfully!"
echo "=========================================="
echo "Output file: $OUTPUT_INITRD"
echo "Size:        $(du -h "$OUTPUT_INITRD" | cut -f1)"
echo "SHA256:      $(sha256sum "$OUTPUT_INITRD" | cut -d' ' -f1)"
echo "=========================================="
