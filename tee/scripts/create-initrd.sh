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
for cmd in sh mount umount mkdir mknod switch_root; do
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

# Debug: Show we're starting
echo "=========================================="
echo "Katana TEE Init - Starting"
echo "=========================================="

# Mount virtual filesystems
echo "[init] Mounting proc..."
/bin/mount -t proc proc /proc || echo "[init] WARNING: Failed to mount proc"

echo "[init] Mounting sysfs..."
/bin/mount -t sysfs sysfs /sys || echo "[init] WARNING: Failed to mount sysfs"

echo "[init] Mounting devtmpfs..."
/bin/mount -t devtmpfs devtmpfs /dev || echo "[init] WARNING: Failed to mount devtmpfs"

# Create essential device nodes if they don't exist
echo "[init] Creating essential device nodes..."
[ -c /dev/null ] || /bin/mknod /dev/null c 1 3
[ -c /dev/console ] || /bin/mknod /dev/console c 5 1

# Setup networking
echo "[init] Setting up loopback interface..."
ip link set lo up 2>/dev/null || echo "[init] WARNING: Failed to setup loopback"

# Show mounted filesystems
echo "[init] Mounted filesystems:"
/bin/mount

# Show available binaries
echo "[init] Available binaries in /bin:"
ls -la /bin/

# Show environment
echo "[init] Environment variables:"
env

# Show kernel command line
echo "[init] Kernel command line:"
cat /proc/cmdline

echo "=========================================="
echo "[init] Launching Katana..."
echo "=========================================="

# Launch Katana with verbose output
exec /bin/katana "$@"
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
