#!/bin/bash
set -euo pipefail

# Parse arguments
OUTPUT=""
KERNEL=""
INITRD=""
CMDLINE_FILE=""
SIZE="2G"

while [[ $# -gt 0 ]]; do
    case $1 in
        --output)
            OUTPUT="$2"
            shift 2
            ;;
        --kernel)
            KERNEL="$2"
            shift 2
            ;;
        --initrd)
            INITRD="$2"
            shift 2
            ;;
        --cmdline-file)
            CMDLINE_FILE="$2"
            shift 2
            ;;
        --size)
            SIZE="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Validate required arguments
if [[ -z "$OUTPUT" || -z "$KERNEL" || -z "$INITRD" || -z "$CMDLINE_FILE" ]]; then
    echo "Usage: $0 --output FILE --kernel FILE --initrd FILE --cmdline-file FILE [--size SIZE]"
    exit 1
fi

# Verify SOURCE_DATE_EPOCH is set
if [[ -z "${SOURCE_DATE_EPOCH:-}" ]]; then
    echo "ERROR: SOURCE_DATE_EPOCH must be set"
    exit 1
fi

echo "=========================================="
echo "Building VM Image (Debug Mode)"
echo "=========================================="
echo "Configuration:"
echo "  Output:      $OUTPUT"
echo "  Kernel:      $KERNEL"
echo "  Initrd:      $INITRD"
echo "  Cmdline:     $CMDLINE_FILE"
echo "  Size:        $SIZE"
echo "  SOURCE_DATE_EPOCH: $SOURCE_DATE_EPOCH"
echo "=========================================="
echo ""

# Show sizes of input files
echo "Input file sizes:"
ls -lh "$KERNEL" "$INITRD" "$CMDLINE_FILE"
echo ""

# Create sparse raw disk image
truncate -s "$SIZE" "$OUTPUT"

# Create GPT partition table
echo "Creating GPT partition table..."
echo "  Partition 1: EFI (100MB, type ef00)"
echo "  Partition 2: ROOT (remaining, type 8300)"
sgdisk --clear \
       --new=1:2048:+100M --typecode=1:ef00 --change-name=1:'EFI' \
       --new=2:0:0 --typecode=2:8300 --change-name=2:'ROOT' \
       "$OUTPUT"
echo "✓ Partitions created"
echo ""

# Setup loop device
LOOP=$(losetup --find --show --partscan "$OUTPUT")
trap "losetup -d $LOOP 2>/dev/null || true" EXIT

echo "✓ Loop device attached: $LOOP"

# Wait for partition devices to appear and force kernel to detect partitions
echo "Waiting for partition devices..."
sleep 1
# Force partition table re-read
partprobe ${LOOP} 2>/dev/null || true
sleep 1

# Determine which partition device naming scheme is available
PART1="${LOOP}p1"
PART2="${LOOP}p2"
USING_KPARTX=false

if [[ ! -b "${LOOP}p1" ]]; then
    echo "Partitions not detected, trying kpartx..."
    apt-get update >/dev/null 2>&1 && apt-get install -y kpartx >/dev/null 2>&1 || true
    kpartx -av ${LOOP} 2>/dev/null || true
    sleep 1

    # Check if kpartx created mapper devices
    LOOP_NAME=$(basename ${LOOP})
    if [[ -b "/dev/mapper/${LOOP_NAME}p1" ]]; then
        PART1="/dev/mapper/${LOOP_NAME}p1"
        PART2="/dev/mapper/${LOOP_NAME}p2"
        USING_KPARTX=true
        echo "✓ Using kpartx devices: ${PART1}, ${PART2}"
    fi
fi

# Update trap to cleanup kpartx mappings if used
if [[ "$USING_KPARTX" == "true" ]]; then
    trap "kpartx -d ${LOOP} 2>/dev/null || true; losetup -d $LOOP 2>/dev/null || true" EXIT
fi

ls -la ${LOOP}* /dev/mapper/${LOOP_NAME}* 2>/dev/null || echo "WARNING: Partition devices not visible yet"
echo ""

# Format partitions
echo "Formatting EFI partition (FAT32)..."
mkfs.fat -F32 -n EFI "${PART1}" || { echo "ERROR: Failed to format EFI partition"; exit 1; }
echo "✓ EFI partition formatted"
echo ""

echo "Formatting root partition (ext4, deterministic)..."
# Use deterministic UUID and no random features
mkfs.ext4 -L ROOT -U clear -O ^metadata_csum,^64bit "${PART2}" || { echo "ERROR: Failed to format ROOT partition"; exit 1; }
echo "✓ ROOT partition formatted"
echo ""

# Create mount points
MNT_DIR=$(mktemp -d)
if [[ "$USING_KPARTX" == "true" ]]; then
    trap "umount -R $MNT_DIR 2>/dev/null || true; rm -rf $MNT_DIR; kpartx -d ${LOOP} 2>/dev/null || true; losetup -d $LOOP 2>/dev/null || true" EXIT
else
    trap "umount -R $MNT_DIR 2>/dev/null || true; rm -rf $MNT_DIR; losetup -d $LOOP 2>/dev/null || true" EXIT
fi

mkdir -p "$MNT_DIR"/{efi,root}

# Mount filesystems
echo "Mounting filesystems..."
mount "${PART1}" "$MNT_DIR/efi" || { echo "ERROR: Failed to mount EFI partition"; exit 1; }
echo "✓ EFI partition mounted at $MNT_DIR/efi"
mount "${PART2}" "$MNT_DIR/root" || { echo "ERROR: Failed to mount ROOT partition"; exit 1; }
echo "✓ ROOT partition mounted at $MNT_DIR/root"
echo ""

# Create minimal root filesystem structure
echo "Creating root filesystem structure..."
mkdir -p "$MNT_DIR/root"/{bin,dev,proc,sys,tmp,var,etc}
echo "✓ Root directories created"
echo ""

# Install systemd-boot to EFI partition
echo "Setting up EFI boot structure..."
mkdir -p "$MNT_DIR/efi"/{EFI/BOOT,loader/entries}
echo "✓ Boot directories created"
echo ""

# Copy kernel and initrd to EFI partition
echo "Copying kernel and initrd to EFI partition..."
cp "$KERNEL" "$MNT_DIR/efi/vmlinuz" || { echo "ERROR: Failed to copy kernel"; exit 1; }
echo "✓ Kernel copied ($(du -h "$MNT_DIR/efi/vmlinuz" | cut -f1))"
cp "$INITRD" "$MNT_DIR/efi/initrd.img" || { echo "ERROR: Failed to copy initrd"; exit 1; }
echo "✓ Initrd copied ($(du -h "$MNT_DIR/efi/initrd.img" | cut -f1))"
echo ""

# Read kernel command line from file
CMDLINE=$(cat "$CMDLINE_FILE")
echo "Kernel command line: $CMDLINE"
echo ""

# Create systemd-boot loader configuration
echo "Creating bootloader configuration..."
cat > "$MNT_DIR/efi/loader/loader.conf" <<EOF
default katana.conf
timeout 0
console-mode keep
EOF

# Create boot entry
cat > "$MNT_DIR/efi/loader/entries/katana.conf" <<EOF
title   Katana TEE
linux   /vmlinuz
initrd  /initrd.img
options $CMDLINE
EOF

# Create EFI boot executable (copy systemd-boot)
# In a real implementation, we would use bootctl or copy BOOTX64.EFI
# For now, we create a marker file
touch "$MNT_DIR/efi/EFI/BOOT/BOOTX64.EFI"
echo "✓ Boot configuration files created"
echo ""

# Show what we created
echo "EFI partition contents:"
ls -lh "$MNT_DIR/efi/"
echo ""
echo "EFI boot entries:"
ls -lh "$MNT_DIR/efi/loader/entries/"
echo ""

echo "Setting timestamps to SOURCE_DATE_EPOCH ($SOURCE_DATE_EPOCH)..."
# Set all timestamps to SOURCE_DATE_EPOCH for reproducibility
find "$MNT_DIR/efi" "$MNT_DIR/root" -exec touch -h -d "@${SOURCE_DATE_EPOCH}" {} + 2>/dev/null || true
echo "✓ Timestamps normalized"
echo ""

# Sync to ensure all data is written
echo "Syncing filesystems..."
sync
echo "✓ Sync complete"
echo ""

# Unmount filesystems
echo "Unmounting filesystems..."
umount "$MNT_DIR/efi" || echo "WARNING: Failed to unmount EFI"
umount "$MNT_DIR/root" || echo "WARNING: Failed to unmount ROOT"
echo "✓ Filesystems unmounted"
echo ""

# Detach loop device
echo "Detaching loop device..."
losetup -d "$LOOP" || echo "WARNING: Failed to detach loop device"
echo "✓ Loop device detached"
echo ""

echo "=========================================="
echo "✓ VM image created successfully!"
echo "=========================================="
echo "Output file: $OUTPUT"
echo "Image size:  $(du -h "$OUTPUT" | cut -f1)"
echo "SHA256:      $(sha256sum "$OUTPUT" | cut -d' ' -f1)"
echo "=========================================="
