# AMD SEV-SNP TEE Build Scripts

Build scripts for creating TEE (Trusted Execution Environment) components to run Katana inside AMD SEV-SNP confidential VMs.

## Requirements

- **QEMU 10.2.0** - Only tested with this version. Earlier versions may lack required SEV-SNP features.
  ```sh
  # Build from source using the provided script
  ./misc/AMDSEV/build-qemu.sh
  ```
- AMD EPYC processor with SEV-SNP support
- Host kernel with SEV-SNP enabled

## Quick Start

```sh
# From repository root - builds everything (OVMF, kernel, katana, initrd)
./misc/AMDSEV/build.sh

# Or with a pre-built katana binary (must be statically linked)
./misc/AMDSEV/build.sh --katana /path/to/katana
```

Output is written to `misc/AMDSEV/output/qemu/`.

### Katana Binary

If `--katana` is not provided, `build.sh` automatically builds a statically linked katana binary using musl libc via `scripts/build-musl.sh`.

**Important:** The initrd is minimal and contains no libc or shared libraries. Only statically linked binaries will work. If providing a custom binary with `--katana`, ensure it is statically linked (e.g., built with musl).

## Scripts

| Script | Description |
|--------|-------------|
| `build.sh` | Main orchestrator - builds all components and generates `build-info.txt` |
| `build-qemu.sh` | Builds QEMU 10.2.0 from source with SEV-SNP support |
| `build-ovmf.sh` | Builds OVMF firmware from AMD's fork with SEV-SNP support |
| `build-kernel.sh` | Downloads and extracts Ubuntu kernel (`vmlinuz`) |
| `build-initrd.sh` | Creates minimal initrd with busybox, SEV-SNP modules, and katana |
| `build-config` | Pinned versions and checksums for reproducible builds |

## Output Files

| File | Description |
|------|-------------|
| `OVMF.fd` | UEFI firmware with SEV-SNP support |
| `vmlinuz` | Linux kernel |
| `initrd.img` | Initial ramdisk containing katana |
| `katana` | Katana binary (copied from build) |
| `build-info.txt` | Build metadata and checksums |

## Running

```sh
qemu-system-x86_64 \
    # Use KVM hardware virtualization (required for SEV-SNP)
    -enable-kvm \
    # AMD EPYC CPU with SEV-SNP support
    -cpu EPYC-v4 \
    # Q35 machine type with confidential computing enabled, referencing sev0 object
    -machine q35,confidential-guest-support=sev0 \
    # SEV-SNP guest configuration:
    #   policy=0x30000    - Guest policy flags (SMT allowed, debug disabled)
    #   cbitpos=51        - C-bit position in page table entries (memory encryption bit)
    #   reduced-phys-bits - Physical address bits reserved for encryption
    #   kernel-hashes=on  - Include kernel/initrd/cmdline hashes in attestation report,
    #                       allowing remote verifiers to confirm exact boot components
    # 
    # Reference: https://www.qemu.org/docs/master/system/i386/amd-memory-encryption.html#launching-sev-snp
    -object sev-snp-guest,id=sev0,policy=0x30000,cbitpos=51,reduced-phys-bits=1,kernel-hashes=on \
    # OVMF firmware with SEV-SNP support (measures itself into attestation)
    -bios output/qemu/OVMF.fd \
    # Direct kernel boot (kernel is measured when kernel-hashes=on)
    -kernel output/qemu/vmlinuz \
    # Initial ramdisk containing katana (measured when kernel-hashes=on)
    -initrd output/qemu/initrd.img \
    # Kernel command line (measured when kernel-hashes=on)
    # katana.args passes arguments to katana via init script
    -append "console=ttyS0 katana.args=--http.addr,0.0.0.0,--http.port,5050,--tee.provider,sev-snp" \
    ..
```

## Reproducible Builds

Set `SOURCE_DATE_EPOCH` for deterministic output:

```sh
SOURCE_DATE_EPOCH=$(git log -1 --format=%ct) ./misc/AMDSEV/build.sh
```

## Troubleshooting

### `SEV: guest firmware hashes table area is invalid (base=0x0 size=0x0)`

**Error:**
```
qemu-system-x86_64: SEV: guest firmware hashes table area is invalid (base=0x0 size=0x0)
```

**Cause:** You are using a standard OVMF firmware (`OvmfPkgX64.dsc`) instead of the AMD SEV OVMF firmware (`AmdSevX64.dsc`) with `kernel-hashes=on`.

When `kernel-hashes=on` is enabled, QEMU needs to inject SHA-256 hashes of the kernel, initrd, and command line into a reserved memory region in the OVMF firmware. The AMD SEV OVMF reserves a 1KB region for this hash table (`PcdQemuHashTableBase=0x010C00`, `PcdQemuHashTableSize=0x000400`), while the standard OVMF has no such region (base=0x0, size=0x0).

**Solution:** Use the OVMF firmware built from `AmdSevX64.dsc`. The `build-ovmf.sh` script already handles building a compatible version from AMD's fork:

```sh
# Use the OVMF built by build.sh or build-ovmf.sh
-bios output/qemu/OVMF.fd

# Or rebuild it manually:
source build-config && ./build-ovmf.sh ./output/qemu
```

Do not use generic OVMF builds from your distribution or other sources when using `kernel-hashes=on` with SEV-SNP.

**Reference:** [AMD's OVMF fork](https://github.com/AMDESE/ovmf) (branch `snp-latest`) contains the SEV-SNP support and hash table memory region required for direct kernel boot with attestation.
