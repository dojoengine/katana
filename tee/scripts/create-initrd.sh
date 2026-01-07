#!/bin/bash
set -euo pipefail

KATANA_BINARY=$1
OUTPUT_INITRD=$2

echo "=========================================="
echo "Creating Initrd (Debug Mode)"
echo "=========================================="
echo "Configuration:"
echo "  Katana binary:       $KATANA_BINARY"
echo "  Output initrd:       $OUTPUT_INITRD"
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
for cmd in sh mount umount mkdir mknod switch_root ip; do
    ln -s busybox bin/$cmd
    echo "  - bin/$cmd -> busybox"
done
echo "✓ Symlinks created"
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

# Parse args for Katana from cmdline (simple convention, no spaces)
KATANA_ARGS=""
for tok in $CMDLINE; do
  case "$tok" in
    katana.args=*)
      KATANA_ARGS="${tok#katana.args=}"
      ;;
  esac
done

dbg "Katana args: $KATANA_ARGS"
log "Launching Katana..."

# shellcheck disable=SC2086
exec /bin/katana $KATANA_ARGS
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
