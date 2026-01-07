# Debug Mode Configuration

This document describes the debugging optimizations made to the Katana TEE VM image build pipeline.

## Overview

All scripts and configurations have been optimized for maximum debugging visibility during development. This makes it easier to:
- Troubleshoot boot issues
- Verify each step of the build process
- See what's happening inside the VM
- Track down problems quickly

## Kernel Command Line (Debug Mode)

**File:** `tee/configs/cmdline.txt`

```
console=ttyS0 loglevel=8 debug earlyprintk=serial,ttyS0,115200
```

### Parameters Explained

| Parameter | Value | Purpose |
|-----------|-------|---------|
| `console=ttyS0` | Serial port | Redirect all output to serial console |
| `loglevel=8` | Maximum | Show ALL kernel messages including debug |
| `debug` | Flag | Enable additional kernel debug output |
| `earlyprintk=serial,ttyS0,115200` | Early console | Show messages before regular console initializes |

**Effect:** You will see EVERY kernel message from the earliest boot stage through to application startup.

## Init Script (Debug Mode)

**File:** `tee/scripts/create-initrd.sh` (generates `/init` inside initrd)

The init script now includes:

### Debug Features

1. **Startup banner**
   ```
   ==========================================
   Katana TEE Init - Starting
   ==========================================
   ```

2. **Step-by-step progress messages**
   - Each mount operation reports success/failure
   - Each setup step shows what's happening
   - Warnings shown for non-critical failures

3. **System information dump**
   - Mounted filesystems (`mount` output)
   - Available binaries (`ls -la /bin/`)
   - Environment variables (`env`)
   - Kernel command line (`/proc/cmdline`)

4. **Clear launch marker**
   ```
   ==========================================
   [init] Launching Katana...
   ==========================================
   ```

### What You'll See

When the VM boots, you'll see output like:

```
==========================================
Katana TEE Init - Starting
==========================================
[init] Mounting proc...
[init] Mounting sysfs...
[init] Mounting devtmpfs...
[init] Creating essential device nodes...
[init] Setting up loopback interface...
[init] Mounted filesystems:
proc on /proc type proc (rw,relatime)
sysfs on /sys type sysfs (rw,relatime)
devtmpfs on /dev type devtmpfs (rw,relatime)
[init] Available binaries in /bin:
total 2048
drwxr-xr-x    2 0        0             4096 Jan  7 19:00 .
drwxr-xr-x   10 0        0             4096 Jan  7 19:00 ..
-rwxr-xr-x    1 0        0          1024000 Jan  7 19:00 busybox
-rwxr-xr-x    1 0        0          1024000 Jan  7 19:00 katana
[init] Environment variables:
PATH=/bin:/sbin:/usr/bin:/usr/sbin
[init] Kernel command line:
console=ttyS0 loglevel=8 debug earlyprintk=serial,ttyS0,115200
==========================================
[init] Launching Katana...
==========================================
```

## Build Scripts (Debug Mode)

All build scripts now include extensive debug output:

### create-initrd.sh Debug Features

**Location:** `tee/scripts/create-initrd.sh`

- Configuration summary at start
- Katana binary verification and info
- Step-by-step progress for each operation
- Directory structure listing
- File size information
- SHA256 hash of output
- Success/failure indicators (✓ or ERROR)

**Example Output:**
```
==========================================
Creating Initrd (Debug Mode)
==========================================
Configuration:
  Katana binary:       /katana-binary
  Output initrd:       /output/initrd.img
  SOURCE_DATE_EPOCH:   1736277600
==========================================

Katana binary info:
-rwxr-xr-x 1 root root 1.0M Jan 7 19:00 /katana-binary
/katana-binary: ELF 64-bit LSB executable, x86-64, statically linked

Creating initrd directory structure...
✓ Directories created

Copying busybox...
✓ Busybox copied from /bin/busybox

Creating busybox symlinks...
  - bin/sh -> busybox
  - bin/mount -> busybox
  [...]
✓ Symlinks created

Copying Katana binary...
✓ Katana copied to bin/katana
-rwxr-xr-x 1 root root 1.0M Jan 7 19:00 bin/katana

[... more output ...]

==========================================
✓ Initrd created successfully!
==========================================
Output file: /output/initrd.img
Size:        512K
SHA256:      a1b2c3d4e5f6...
==========================================
```

### build-vm-image.sh Debug Features

**Location:** `tee/scripts/build-vm-image.sh`

- Configuration summary with all parameters
- Input file sizes
- Partition creation details
- Loop device information
- Filesystem format progress
- Mount points and status
- File copy operations with sizes
- Kernel command line display
- EFI partition contents listing
- Timestamp normalization status
- Final image hash

**Example Output:**
```
==========================================
Building VM Image (Debug Mode)
==========================================
Configuration:
  Output:      /output/disk.raw
  Kernel:      /components/vmlinuz
  Initrd:      /components/initrd.img
  Cmdline:     /configs/cmdline.txt
  Size:        2G
  SOURCE_DATE_EPOCH: 1736277600
==========================================

Input file sizes:
-rw-r--r-- 1 root root 10M Jan 7 19:00 /components/vmlinuz
-rw-r--r-- 1 root root 512K Jan 7 19:00 /components/initrd.img
-rw-r--r-- 1 root root 64 Jan 7 19:00 /configs/cmdline.txt

Creating GPT partition table...
  Partition 1: EFI (100MB, type ef00)
  Partition 2: ROOT (remaining, type 8300)
✓ Partitions created

✓ Loop device attached: /dev/loop0

Waiting for partition devices...
brw-rw---- 1 root disk 7, 0 Jan  7 19:00 /dev/loop0
brw-rw---- 1 root disk 259, 0 Jan  7 19:00 /dev/loop0p1
brw-rw---- 1 root disk 259, 1 Jan  7 19:00 /dev/loop0p2

Formatting EFI partition (FAT32)...
✓ EFI partition formatted

Formatting root partition (ext4, deterministic)...
✓ ROOT partition formatted

[... more output ...]

==========================================
✓ VM image created successfully!
==========================================
Output file: /output/disk.raw
Image size:  2.0G
SHA256:      a1b2c3d4e5f6...
==========================================
```

## Viewing Debug Output

### During Docker Build

When building the VM image with Docker, you'll see all this debug output in real-time:

```bash
docker build -f vm-image.Dockerfile \
  --build-arg SOURCE_DATE_EPOCH=$(git log -1 --format=%ct) \
  --build-context katana-binary=. \
  -t katana-vm-image .
```

### During VM Boot

When booting the VM with QEMU, connect to the serial console to see all boot messages:

```bash
qemu-system-x86_64 \
  -enable-kvm \
  -m 4G \
  -cpu EPYC-v4 \
  -bios ovmf.fd \
  -drive format=raw,file=disk.raw \
  -nographic \
  -serial mon:stdio
```

You'll see:
1. OVMF firmware messages
2. Kernel boot messages (very verbose with loglevel=8)
3. Init script debug output
4. Katana startup

### Via GCP Serial Console

For VMs running in GCP Confidential Computing:

```bash
gcloud compute instances get-serial-port-output INSTANCE_NAME
```

Or connect interactively:
```bash
gcloud compute connect-to-serial-port INSTANCE_NAME
```

## Debug vs Production

### Current (Debug) Configuration

**Optimized for:** Development, troubleshooting, testing

**Characteristics:**
- Maximum verbosity
- Detailed progress messages
- System information dumps
- Early boot messages
- Every step logged

**Trade-offs:**
- Slower boot (serial console is slow)
- More console output = more data
- Potential information leakage
- Larger logs

### Production Configuration

When ready for production, change to:

#### cmdline.txt (Production)
```
console=ttyS0 loglevel=3 ro
```

#### Init Script (Production)
Remove or reduce debug messages:
- Keep critical error messages
- Remove info dumps (mount, env, etc.)
- Remove progress indicators
- Keep only what's needed for troubleshooting real issues

**Benefits:**
- Faster boot
- Less information exposure
- Smaller logs
- More professional appearance

## Switching Modes

### To Production Mode

1. **Update cmdline.txt:**
   ```bash
   echo "console=ttyS0 loglevel=3 ro" > tee/configs/cmdline.txt
   ```

2. **Simplify init script** in `create-initrd.sh`:
   - Remove echo statements for progress
   - Keep only error messages
   - Remove info dumps (mount, ls, env)

3. **Simplify build scripts** (optional):
   - Remove "✓" indicators
   - Keep only error messages
   - Remove detailed listings

### Back to Debug Mode

1. **Update cmdline.txt:**
   ```bash
   echo "console=ttyS0 loglevel=8 debug earlyprintk=serial,ttyS0,115200" > tee/configs/cmdline.txt
   ```

2. Init script and build scripts are already in debug mode (no changes needed if you kept the debug versions)

## Debugging Tips

### If the VM doesn't boot

1. **Check QEMU output** - Look for the last message before it stops
2. **Try even more verbose kernel:** `loglevel=9`
3. **Add init debug:** Change init to `/bin/sh` to drop to a shell:
   ```
   init=/bin/sh
   ```

### If Katana fails to start

The init script will show:
- What files are available
- What the environment looks like
- The exact command line being used

### If you can't see any output

1. **Check console parameter** - Make sure `console=ttyS0` is present
2. **Check QEMU serial setup** - Use `-serial mon:stdio` or `-serial file:serial.log`
3. **Try VGA console** - Add `console=tty0 console=ttyS0` (both)

## Current Status

✅ **All debug optimizations complete:**
- Kernel: Maximum verbosity + early boot messages
- Init: Full system information dump + progress tracking
- Build scripts: Comprehensive step-by-step output with success indicators
- All scripts show SHA256 hashes for verification

This configuration gives you complete visibility into every aspect of the build and boot process, making debugging much easier during development.
