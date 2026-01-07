# VM Image Build Process

This document provides technical details about the Katana TEE VM image build pipeline.

## Overview

The VM image build creates a bootable virtual machine image containing the Katana binary for deployment in AMD SEV-SNP Trusted Execution Environments (TEEs). The build process is:

- **Reproducible**: Identical inputs produce bit-for-bit identical outputs
- **Attestable**: All components have cryptographic hashes and GitHub attestations
- **Minimal**: Only essential components included for reduced attack surface

## Architecture

```
┌──────────────────┐
│ Reproducible     │  Static musl binary
│ Binary Build     │  (x86_64-unknown-linux-musl)
└────────┬─────────┘
         │
         ├─────────────────────────────────────┐
         │                                     │
┌────────▼─────────┐              ┌───────────▼──────────┐
│   VM Image       │              │   Measurement        │
│   Builder        │              │   Calculator         │
│                  │              │                      │
│  ├─ Kernel       │              │  ├─ sev-snp-measure  │
│  ├─ Initrd       │──────────────┤  ├─ OVMF firmware    │
│  ├─ OVMF         │              │  ├─ Kernel           │
│  └─ Disk Image   │              │  └─ Initrd           │
└──────────────────┘              └──────────────────────┘
         │                                     │
         │                                     │
         ▼                                     ▼
    Artifacts                          Expected Measurement
```

## Build Components

### 1. Reproducible Binary (`reproducible.Dockerfile`)

**Location**: `/reproducible.Dockerfile`

Creates a static Katana binary with:
- Rust 1.86.0 (pinned by SHA256)
- musl libc for static linking
- Fat LTO optimization
- SOURCE_DATE_EPOCH for reproducible timestamps
- Vendored dependencies for offline builds

**Output**: `/katana` (58MB static binary)

### 2. VM Image Builder (`vm-image.Dockerfile`)

**Location**: `/vm-image.Dockerfile`

Multi-stage Dockerfile that assembles the VM image:

#### Stage 1: Package Fetcher
- Downloads Ubuntu 24.04 packages from archive.ubuntu.com
- Pins exact versions:
  - `linux-image-6.8.0-90-generic`
  - `ovmf` (2024.02-1)
  - `busybox-static`

#### Stage 2: Component Builder
- Extracts kernel from `.deb` packages
- Extracts OVMF firmware
- Prepares busybox for initrd

#### Stage 3: Initrd Builder
- Embeds Katana binary in initrd
- Creates minimal init script
- Includes busybox for shell and utilities
- Adds network configuration
- Generates compressed cpio archive

**Script**: `/tee/scripts/create-initrd.sh`

#### Stage 4: Image Builder
- Creates raw disk image (2GB)
- GPT partition table
- EFI system partition (FAT32)
- Root filesystem (ext4)
- Installs systemd-boot
- Copies kernel and initrd

**Script**: `/tee/scripts/build-vm-image.sh`

#### Final Stage
- Exports all components:
  - `disk.raw` - Bootable disk image
  - `vmlinuz` - Linux kernel
  - `initrd.img` - Initial RAM disk with Katana
  - `ovmf.fd` - OVMF firmware

### 3. Measurement Calculator

**Script**: `/tee/scripts/calculate-measurement.sh`

Calculates the expected SEV-SNP measurement using the `sev-snp-measure` Python tool:

```bash
./tee/scripts/calculate-measurement.sh \
    ovmf.fd \
    vmlinuz \
    initrd.img \
    "console=ttyS0 katana.args=--http.addr=0.0.0.0" \
    4 \
    EPYC-v4
```

**Output**:
- `expected-measurement.txt` - Hex measurement (96 chars)
- `expected-measurement.json` - Structured metadata

## Initrd Contents

The initrd is the core of the TEE environment and contains:

```
initrd/
├── bin/
│   ├── busybox       # Shell and basic utilities
│   ├── ip            # Network configuration
│   ├── katana        # Katana binary (58MB)
│   ├── mount
│   ├── sh
│   └── ...
├── dev/              # Device nodes
├── proc/             # Process filesystem mount point
├── sys/              # Sysfs mount point
├── tmp/              # Temporary files
├── etc/
│   ├── passwd        # Minimal user database
│   └── group         # Minimal group database
└── init              # Init script (PID 1)
```

### Init Script Flow

1. Mount `/proc` first
2. Read kernel command line from `/proc/cmdline`
3. Mount `/sys` and `/dev`
4. Create essential device nodes
5. Configure network (eth0 + loopback)
6. Parse Katana arguments from cmdline
7. `exec` Katana as PID 1

## Reproducibility Measures

### 1. Timestamps
All file timestamps set to `SOURCE_DATE_EPOCH` (git commit time):

```bash
export SOURCE_DATE_EPOCH=$(git log -1 --format=%ct)
find . -exec touch -h -d "@${SOURCE_DATE_EPOCH}" {} +
```

### 2. Deterministic Ordering
- Cpio archives created with sorted filenames
- `LC_ALL=C sort -z` for locale-independent sorting

### 3. Pinned Dependencies
- Base images by SHA256 digest
- Ubuntu packages by exact version
- Rust toolchain by exact version

### 4. No Randomness
- Fixed partition UUIDs
- Fixed filesystem labels
- No timestamps in compressed archives (`gzip -n`)

### 5. Build Environment
```bash
TZ=UTC
LANG=C.UTF-8
LC_ALL=C.UTF-8
```

## CI/CD Integration

The build is integrated into `.github/workflows/release-tee.yml`:

### Jobs

1. **prepare** - Determines version tag
2. **build-contracts** - Compiles Starknet contracts
3. **reproducible-build** - Builds static Katana binary
4. **vm-image-build** - Builds VM image (this document)

### vm-image-build Steps

1. Download reproducible binary artifact
2. Build VM image with Docker
3. Extract components
4. Install `sev-snp-measure` tool
5. Calculate measurement
6. Generate manifest
7. Create GitHub attestation
8. Upload artifacts

### Artifacts

- `katana-vm-{version}.raw.gz` - Compressed disk image
- `expected-measurement.json` - SEV-SNP measurement
- `manifest.json` - Component hashes
- `vmlinuz` - Kernel
- `initrd.img` - Initrd
- `ovmf.fd` - OVMF firmware

## Local Build

### Prerequisites

```bash
# Install Docker
sudo apt-get install docker.io

# Install sev-snp-measure
pipx install sev-snp-measure

# Install QEMU for testing
sudo apt-get install qemu-system-x86
```

### Build Steps

```bash
# 1. Build reproducible binary
export SOURCE_DATE_EPOCH=$(git log -1 --format=%ct)
docker build \
    -f reproducible.Dockerfile \
    -t katana-reproducible:local \
    --build-arg SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH} \
    .

# 2. Extract binary
docker create --name katana-extract katana-reproducible:local
docker cp katana-extract:/katana ./katana
docker rm katana-extract

# 3. Build VM image
docker build \
    -f vm-image.Dockerfile \
    -t katana-vm:local \
    --build-arg SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH} \
    --build-arg KATANA_BINARY=./katana \
    .

# 4. Extract VM components
docker create --name vm-extract katana-vm:local
docker cp vm-extract:/output/disk.raw ./disk.raw
docker cp vm-extract:/output/vmlinuz ./vmlinuz
docker cp vm-extract:/output/initrd.img ./initrd.img
docker cp vm-extract:/output/ovmf.fd ./ovmf.fd
docker rm vm-extract

# 5. Calculate measurement
./tee/scripts/calculate-measurement.sh \
    ovmf.fd \
    vmlinuz \
    initrd.img \
    "console=ttyS0" \
    4 \
    EPYC-v4
```

## Testing

### Boot Test with QEMU

```bash
# Test VM boot
./tee/scripts/test-vm-boot.sh

# Or manually:
qemu-system-x86_64 \
    -m 4G \
    -smp 4 \
    -kernel vmlinuz \
    -initrd initrd.img \
    -append "console=ttyS0 katana.args=--http.addr=0.0.0.0" \
    -nographic \
    -net nic,model=virtio \
    -net user,hostfwd=tcp::5050-:5050
```

### Test RPC Endpoint

```bash
# Health check
curl http://localhost:5050/

# Chain ID
curl -X POST http://localhost:5050 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"starknet_chainId","params":[],"id":1}'
```

## Security Considerations

### Minimal Attack Surface

- No SSH server
- No unnecessary packages
- Single-purpose init (just launches Katana)
- Read-only root filesystem
- Firewall rules (future enhancement)

### Network Configuration

- Loopback interface for localhost
- eth0 with static IP (10.0.2.15) for QEMU user networking
- No external network exposure by default

### Measured Boot

All components included in SEV-SNP measurement:
- OVMF firmware
- Kernel
- Initrd (includes Katana binary)
- Kernel command line

## Troubleshooting

### Build Failures

**Error**: `Initramfs unpacking failed: write error`
- **Cause**: Insufficient RAM for 60MB initrd
- **Fix**: Increase QEMU memory to 4GB (`-m 4G`)

**Error**: `Kernel panic - not syncing: Attempted to kill init!`
- **Cause**: Init script error (check with `set -eu`)
- **Fix**: Examine serial log, ensure all commands have error handling

**Error**: `RPC not responding`
- **Cause**: Network not configured or Katana listening on wrong address
- **Fix**: Ensure init script configures network and Katana uses `--http.addr=0.0.0.0`

### Measurement Calculation

**Error**: `OVMF metadata doesn't include SNP_KERNEL_HASHES`
- **Cause**: OVMF firmware doesn't support direct kernel boot measurement
- **Fix**: Script automatically falls back to UEFI-only measurement (expected behavior)

## Performance

### Build Times

- Reproducible binary: ~5-10 minutes
- VM image: ~3-5 minutes
- Total pipeline: ~15-20 minutes

### Artifact Sizes

- Katana binary: 58MB
- Initrd (compressed): 21MB
- Initrd (uncompressed): 60MB
- Kernel: 15MB
- OVMF: 3.5MB
- Disk image (compressed): ~50MB
- Disk image (raw): 2GB

## References

- [AMD SEV-SNP Spec](https://www.amd.com/system/files/TechDocs/56860.pdf)
- [sev-snp-measure](https://github.com/virtee/sev-snp-measure)
- [Reproducible Builds](https://reproducible-builds.org/)
- [SOURCE_DATE_EPOCH](https://reproducible-builds.org/docs/source-date-epoch/)
