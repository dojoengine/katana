# AMD SEV-SNP TEE Build Scripts

Build scripts for creating TEE (Trusted Execution Environment) components to run Katana inside AMD SEV-SNP confidential VMs.

## Requirements

- **QEMU 10.2.0** - Only tested with this version. Earlier versions may lack required SEV-SNP features.
  ```sh
  # Build from source using the provided script
  ./scripts/build-qemu.sh
  ```
- AMD EPYC processor with SEV-SNP support
- Host kernel with SEV-SNP enabled

## Quick Start

```sh
# Build everything (OVMF, kernel, initrd) with a prebuilt katana binary.
# Download katana from https://github.com/dojoengine/katana/releases.
./build.sh --katana /path/to/katana
```

Output is written to `output/qemu/`.

### Katana Binary

`--katana` is required when building the initrd. Download a prebuilt linux-gnu binary from [dojoengine/katana releases](https://github.com/dojoengine/katana/releases) — release images embed the `*_linux_amd64_native.tar.gz` cairo-native build (the portable `*_linux_amd64.tar.gz` build also works, but the guest then lacks `--enable-native-compilation` support).

For reproducibility, the initrd does not copy glibc or shared libraries from the build host. Instead, `scripts/build-initrd.sh` downloads the exact runtime `.deb` packages listed in `build-config`, verifies their SHA-256 checksums, then copies the ELF interpreter and the shared libraries declared by Katana with `readelf`. If providing a custom dynamic binary with `--katana`, build it against a glibc compatible with the pinned runtime and make sure any extra shared libraries it needs are covered by `GLIBC_RUNTIME_PACKAGES` and `GLIBC_RUNTIME_PACKAGE_SHA256S`.

## Layout

| Path | Description |
|------|-------------|
| `build.sh` | Orchestrator entry point — builds OVMF, kernel, initrd; writes `build-info.txt` |
| `start-vm.sh` | Starts a TEE VM with SEV-SNP and launches Katana asynchronously (consumer-facing) |
| `verify-build.sh` | Verifies sha256s + sealed launch measurement of a build / downloaded release |
| `reproduce-release.sh` | Rebuilds a published release from source and compares it byte-for-byte against the published artifacts |
| `build-config` | Pinned versions and checksums for reproducible builds |
| `scripts/build-ovmf.sh` | Builds OVMF firmware from AMD's fork with SEV-SNP support |
| `scripts/build-kernel.sh` | Downloads and extracts Ubuntu kernel (`vmlinuz`) |
| `scripts/build-initrd.sh` | Creates minimal initrd with busybox, SEV-SNP modules, snp-derivekey, cryptsetup, ld (cairo-native runtime linker), and katana |
| `scripts/build-cryptsetup.sh` | Builds static cryptsetup + mkfs.ext2 in an Alpine container |
| `scripts/build-binutils-ld.sh` | Builds a static GNU ld in the same Alpine container (cairo-native links AOT-compiled classes with it in-guest) |
| `scripts/build-qemu.sh` | Builds QEMU 10.2.0 from source with SEV-SNP support (operator host setup, not part of build pipeline) |
| `scripts/sealed-cmdline.sh` | Single source of truth for the measured kernel cmdline |
| `scripts/test-initrd.sh` | Isolated initrd boot smoke test in plain QEMU |
| `scripts/test-snp-e2e.sh` | End-to-end test on SEV-SNP hardware: sealed boot, attestation vs expected measurement, reboot reseal |
| `snp-tools/` | Cargo crate with `snp-digest`, `snp-report`, `ovmf-metadata`, `snp-derivekey` |
| `docs/release-pipeline.md` | How releases are built, measured, and published — see [Release Pipeline](docs/release-pipeline.md) |

## SNP Tools

The `snp-tools` crate provides CLI utilities for SEV-SNP development:

| Binary | Description |
|--------|-------------|
| `snp-digest` | Calculate SEV-SNP launch measurement digest |
| `snp-report` | Decode and display SEV-SNP attestation reports |
| `ovmf-metadata` | Extract and display OVMF SEV metadata sections |
| `snp-derivekey` | Derive a sealed-storage key via `SNP_GET_DERIVED_KEY` (runs in-guest; used by the initrd to unlock the LUKS data disk) |

Build with:
```sh
cargo build -p snp-tools
```

## Output Files

| File | Description |
|------|-------------|
| `OVMF.fd` | UEFI firmware with SEV-SNP support |
| `vmlinuz` | Linux kernel |
| `initrd.img` | Initial ramdisk containing katana |
| `katana` | Katana binary (copied from build) |
| `build-info.txt` | Build metadata and checksums |

## Running

`start-vm.sh` launches a TEE VM with SEV-SNP enabled and starts Katana inside it.
The three measured boot components (OVMF, kernel, initrd) are **required** and
named explicitly — there is no default boot directory:

```sh
# Minimal boot — points at a fresh build under output/qemu/
sudo ./start-vm.sh \
  --ovmf   output/qemu/OVMF.fd \
  --kernel output/qemu/vmlinuz \
  --initrd output/qemu/initrd.img

# Same boot, also customizing Katana runtime flags (comma-separated)
sudo ./start-vm.sh \
  --ovmf   output/qemu/OVMF.fd \
  --kernel output/qemu/vmlinuz \
  --initrd output/qemu/initrd.img \
  --katana-args "--http.addr,0.0.0.0,--http.port,5050,--tee,sev-snp,--dev"

# Or passing a chain config directory (forwarded to Katana as --chain via a
# read-only virtio-blk ext2 disk packed from the dir contents at boot)
sudo ./start-vm.sh \
  --ovmf   output/qemu/OVMF.fd \
  --kernel output/qemu/vmlinuz \
  --initrd output/qemu/initrd.img \
  --chain-dir /path/to/chain-config

# Or booting without starting Katana (drive the control channel manually)
sudo ./start-vm.sh \
  --ovmf   output/qemu/OVMF.fd \
  --kernel output/qemu/vmlinuz \
  --initrd output/qemu/initrd.img \
  --no-start
```

Why explicit instead of a default dir: each of OVMF / vmlinuz / initrd is hashed
into the SEV-SNP launch measurement, so the operator's intent about which exact
file ends up in the digest should be visible at the invocation site, not hidden
behind a filename + colocation convention. It also makes reproducibility audits
(swap one file against an otherwise-pinned set) tractable without symlink
choreography on the host.

The script:
- Starts QEMU with SEV-SNP confidential computing enabled
- Uses direct kernel boot with `kernel-hashes=on` for attestation
- Creates (on first run) and attaches a persistent data disk as `/dev/sda` — default `~/.katana/data.img`, override with `--data-disk` or `$KATANA_DATA_DISK`
- Boots with **unsealed storage by default**: plain ext4 on `/dev/sda`, cmdline `console=ttyS0`. See [Storage sealing: why unsealed is the default](#storage-sealing-why-unsealed-is-the-default) for the rationale
- With `--sealed`, opts into sealed storage: the data disk is wrapped in LUKS2 + dm-integrity and unlocked inside the guest via `SNP_GET_DERIVED_KEY`. The measured kernel cmdline becomes `console=ttyS0 KATANA_EXPECTED_LUKS_UUID=<uuid>` (a different, separately pinnable launch measurement); the UUID is generated once per host, persisted at `~/.katana/luks-uuid`, and can be overridden with `--luks-uuid` or `$KATANA_LUKS_UUID`
- Delivers Katana's launch configuration via two host-supplied boot-time channels — neither is part of the launch measurement, so changing args or chain config does not change the measured boot:
  - **CLI args via QEMU fw_cfg** at `opt/org.katana/args`. Small payload; fw_cfg's port-I/O sysfs path is fine here.
  - **Chain config via a read-only virtio-blk ext2 disk** built from `--chain-dir` and attached at boot; the guest mounts it at `/run/katana-chain` and passes it to Katana as `--chain`. This used to ride fw_cfg too, but the upstream `qemu_fw_cfg` driver re-reads the whole blob on every sysfs read, making it O(blob²) port I/O and unusable for multi-MB chain configs under SEV-SNP. virtio-blk goes through DMA (SWIOTLB bounce buffers under SNP) and finishes in milliseconds.
  
  The guest treats both channels as untrusted operator input and strips flags init owns (`--db-*`, `--data-dir`, `--chain`).
- Starts Katana asynchronously via a virtio-serial control channel (`start` takes no arguments — config comes from the boot-time channels above)
- Forwards RPC port 5050 to host port 15051
- Outputs serial log to a temp file and follows it

### Manual QEMU Invocation

For reference, this is roughly what `start-vm.sh` runs under the hood. The inline comments make it non-copy-pasteable; see `start-vm.sh` for the exact invocation.

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
    # Kernel command line (measured when kernel-hashes=on).
    # Unsealed variant shown; the sealed (default) variant appends
    # KATANA_EXPECTED_LUKS_UUID=<uuid> (see scripts/sealed-cmdline.sh)
    -append "console=ttyS0" \
    # Data disk, attached as /dev/sda — REQUIRED, the guest init panics
    # without it. Unsealed mode expects an ext4 filesystem on the raw disk;
    # sealed mode expects raw (luksFormat'd on first boot) or LUKS2
    -device virtio-scsi-pci,id=scsi0 \
    -drive file=$HOME/.katana/data.img,format=raw,if=none,id=disk0,cache=none \
    -device scsi-hd,drive=disk0,bus=scsi0.0 \
    # Katana control channel (used to start Katana asynchronously after boot)
    -device virtio-serial-pci,id=virtio-serial0 \
    -chardev socket,id=katanactl,path=/tmp/katana-control.sock,server=on,wait=off \
    -device virtserialport,chardev=katanactl,name=org.katana.control.0 \
    # Katana CLI args via fw_cfg — one per line in the args file. Read by
    # the guest at runtime, NOT part of the launch measurement.
    -fw_cfg name=opt/org.katana/args,file=/path/to/katana-args.txt \
    # Chain config as a read-only virtio-blk ext2 disk. start-vm.sh packs
    # --chain-dir into a small ext2 image with mkfs.ext2 -d at boot. The
    # guest mounts /dev/vda read-only and passes it to Katana as --chain.
    # Also NOT part of the launch measurement.
    -drive file=/path/to/chain.img,format=raw,if=none,id=chaincfg,readonly=on \
    -device virtio-blk-pci,drive=chaincfg,serial=katana-chain \
    ..
```

### Start Katana via Control Channel

In the QEMU example above, this line defines the host-side control channel endpoint:

```sh
-chardev socket,id=katanactl,path=/tmp/katana-control.sock,server=on,wait=off
```

The `path=/tmp/katana-control.sock` value is the Unix socket file on the host
(`start-vm.sh` uses `/tmp/katana-tee-vm-control.<pid>.sock` and prints the path at startup).
That socket is connected to the guest virtio-serial port:

```sh
-device virtserialport,chardev=katanactl,name=org.katana.control.0
```

So writes to that Unix socket become control commands inside the VM:

| Command | Responses |
|---------|-----------|
| `start` | `ok started pid=<pid>`, `err already-running pid=<pid>`, `err start-takes-no-args …` |
| `status` | `running pid=<pid>`, `stopped exit=<code>` |
| `stop` | `ok stopping` — then the guest tears down and powers off |

`stop` is the graceful shutdown path — it stops Katana (TERM, then KILL),
syncs and unmounts the sealed data disk, closes the LUKS mapping, and powers
off, so recent writes are durable across restarts. `start-vm.sh` sends it
automatically on exit before falling back to killing QEMU. Stopping the VM
any other way is a power cut: the LUKS/dm-integrity layers survive it, but
database state still in the guest page cache does not.

`start` takes no arguments: Katana's CLI args and chain config are read once
at boot from the host-supplied boot-time channels (fw_cfg + virtio-blk chain
disk). A `start` with a payload (the old `start <comma-separated-args>`
protocol) is rejected.

Example:

```sh
# Start Katana. Keep stdin open briefly after the command: if socat closes
# the socket as soon as stdin EOFs, QEMU drops the guest's reply (the
# command itself still executes)
{ printf 'start\n'; sleep 2; } | socat -t 2 - UNIX-CONNECT:/tmp/katana-control.sock

# Check launcher status
{ printf 'status\n'; sleep 2; } | socat -t 2 - UNIX-CONNECT:/tmp/katana-control.sock
```

The guest always pins Katana's database to the data disk mount by passing its
own `--db-dir`, mounts the host-supplied chain config disk read-only at
`/run/katana-chain` and passes that as `--chain`, and strips `--db-*` /
`--data-dir` / `--chain` from the host-supplied args.

## Isolated Initrd Testing

Use `test-initrd.sh` for focused initrd boot validation without the full SEV-SNP launch path:

```sh
# Run plain-QEMU boot smoke test
./scripts/test-initrd.sh

# Custom timeout/output directory
./scripts/test-initrd.sh --output-dir ./output/qemu --timeout 300
```

## End-to-End Testing on SNP Hardware

On an SEV-SNP machine, `test-snp-e2e.sh` runs the full trust story as one
command: sealed boot via `start-vm.sh`, RPC liveness, a hardware attestation
quote compared against the expected launch measurement, and a reboot that
must re-open the sealed disk and find the persisted chain state. The same
script backs the `amdsev-snp-e2e` CI workflow, which runs it against every published
release.

```sh
# Test the latest published release
sudo ./scripts/test-snp-e2e.sh

# Test a specific release
sudo ./scripts/test-snp-e2e.sh --tag tee-vm-v0.1.0+katana-v1.8.0-rc.5

# Test a local build before tagging (expected measurement computed with
# snp-digest if built, otherwise the comparison is skipped with a warning)
sudo ./scripts/test-snp-e2e.sh --boot-dir ./output/qemu
```

## Launch Measurement

The launch measurement is a SHA-384 digest computed by the AMD Secure Processor
over the guest's entire initial state at launch. It is the root of trust for
this project, in two ways:

1. **Attestation** — the digest is signed into every SEV-SNP attestation
   report, so a remote verifier can confirm exactly which firmware, kernel,
   initrd, and cmdline the VM booted.
2. **Sealed storage** — the disk-unsealing key is derived inside the guest via
   `SNP_GET_DERIVED_KEY` bound to `MEASUREMENT | GUEST_POLICY`, so changing any
   measured byte produces a different key and the existing data disk no longer
   unseals.

Each release publishes its measurement as `launch-measurement-<tag>.txt`,
computed against the canonical `KATANA_CANONICAL_LUKS_UUID` from `build-config`.

### Storage sealing: why unsealed is the default

`start-vm.sh` boots **unsealed** by default; sealed storage is opt-in via `--sealed`.
The short version: the sealed-storage key is bound to the launch measurement, which
includes the Katana binary, so a version bump re-keys the disk and the old data no
longer unseals — and the measurement-independent ways to avoid that either don't
hold against an untrusted host or add a KMS to the TCB. The default unsealed boot
has no such limitation (a newer Katana opens an older database normally).

Full reasoning, the two-layer (sealing vs `katana-db` format) view, and the
decoupling options A–D (stable identity fields / attestation-gated KMS / wrapped-DEK
re-key ceremony / unsealed) are in the architecture doc:
[`docs/amdsev.md` → Sealed storage](../../docs/amdsev.md#forward-compatibility-and-the-key-binding-limitation).
The sealed path is fully supported and exercised by `scripts/test-snp-e2e.sh`; until
one of options A–C lands, treat a Katana upgrade under `--sealed` as a fresh disk.

### What is measured

| Input | Source | How it enters the digest |
|---|---|---|
| OVMF firmware (`OVMF.fd`) | AMD's fork, pinned commit in `build-config` | Entire firmware image as loaded into guest memory |
| Kernel (`vmlinuz`) | Pinned Ubuntu kernel `.deb` | SHA-256 entry in the SEV hashes table (`kernel-hashes=on`) |
| Initrd (`initrd.img`) | `scripts/build-initrd.sh`, reproducible | SHA-256 entry in the hashes table |
| Kernel cmdline | `scripts/sealed-cmdline.sh` | SHA-256 entry in the hashes table |
| vCPU count and model | `start-vm.sh`: 1 × `EPYC-v4` | Each vCPU's initial register state (VMSA) is measured |
| Guest features | `sev-snp-guest` object: `0x1` (SNP active) | Field in the measured VMSA |
| VMM type | QEMU | VMSA layout differs per VMM |

Two points deserve emphasis:

- **The initrd hash transitively pins everything inside it**: the katana
  binary, busybox, the glibc runtime, kernel modules, cryptsetup,
  snp-derivekey, ld and its libc link inputs, and the init script itself —
  including init's security
  behavior (pinning `--db-dir`/`--chain`, stripping reserved flags from
  operator input). Because the initrd build is reproducible (see
  [Reproducible Builds](#reproducible-builds)), anyone can rebuild it from
  source and arrive at the same hash.
- **The cmdline has two pinnable variants**: sealed boot measures
  `console=ttyS0 KATANA_EXPECTED_LUKS_UUID=<uuid>`; unsealed boot measures
  `console=ttyS0`. They produce different digests — verifiers must pin the
  sealed variant for production use and treat the LUKS UUID as part of the
  expected measurement.

### What is deliberately NOT measured

| Input | Why it stays out |
|---|---|
| Katana CLI args (fw_cfg `opt/org.katana/args`) and chain config (read-only virtio-blk ext2 disk built from `--chain-dir`) | Runtime operator configuration — changing args or chain spec must not re-key the sealed disk or invalidate pinned measurements. The guest treats both channels as untrusted and strips flags init owns (`--db-*`, `--data-dir`, `--chain`). A verifier therefore cannot tell from the report alone which args/chain config Katana runs with. |
| Data disk contents | Protected by a different mechanism: LUKS2 + dm-integrity, with the key derived from the measurement itself — only the measured image can unseal the disk. |
| Guest policy (`0x30000`) | Not an input to the digest, but signed as its own field in the attestation report; verifiers must check it alongside the measurement (it gates debug access and SMT). |
| Host software (QEMU, host kernel, hypervisor) | Untrusted by design under SEV-SNP — the hardware attests the guest without trusting the host. |

### Verifying a measurement

To verify a TEE VM's integrity, compute the expected launch measurement using `snp-digest`:

```sh
# Build the SNP tools
cargo build -p snp-tools

# Sealed boot (start-vm.sh default): the measured cmdline carries the
# per-host LUKS UUID from ~/.katana/luks-uuid
./target/debug/snp-digest \
    --ovmf output/qemu/OVMF.fd \
    --kernel output/qemu/vmlinuz \
    --initrd output/qemu/initrd.img \
    --append "console=ttyS0 KATANA_EXPECTED_LUKS_UUID=$(cat ~/.katana/luks-uuid)" \
    --vcpus 1 \
    --cpu epyc-v4 \
    --vmm qemu \
    --guest-features 0x1

# Unsealed boot (start-vm.sh --unsealed): same command with
#   --append "console=ttyS0"
```

`start-vm.sh` prints the exact `snp-digest` command for its configuration at startup.
The computed measurement should match the `measurement` field in the attestation report.

## Decoding Attestation Reports

Katana running inside a TEE exposes an RPC endpoint to retrieve attestation reports:

```sh
curl -X POST http://localhost:15051 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tee_generateQuote","params":[null,0]}'
```

Params are `[prevBlockId, blockId]`; pass `null` as `prevBlockId` for the genesis block.

Example response (abridged — the exact field set depends on the Katana version):
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "quote": "0x05000000000000000000030000000000000000000000000000000000000000000000000000000000000000000000000001000000010000000a000000000018546700000000000000000000000000000005e1f35913fe09ee5c672a1f1f941dbef203852e79fd118afe9fc09c0e2c242d0000000000000000000000000000000000000000000000000000000000000000a61905c576e54ec9ac77f55ccbc2200eefa5b0613139700ebf16984517634cf14e054792b45ff1a3f4af8922be06d09c000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000023dbf8e0935cca11f2b9bdb518f313296eaa53d743c340b90c342fd4fb8eaaffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff0a0000000000185419110100000000000000000000000000000000000000000001b884fdb43aeab96927fda3a7675bc1d679ca24cde425f6f1c4975749888d3a12aa09535f6eb816553af6d59e278da7e2912acecbc657db12612423f85efd140a000000000018542a3701002a3701000a000000000018540000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000008468441401989b660464e8e9e2643a38cc397fef0f3c302d30600c6409e0f286f6011aad3013ef48a337f6fdd93142c70000000000000000000000000000000000000000000000003cadd7de8ed2fbea2b6f29fa46962d2ce1ed8e5451a1745ee288508648e42182f839e90312797af942c601f9080d3c7d0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
    "stateRoot": "0x1d89a119a324817db2eeee4b68ab886d40ef1f6812768882db55c4b82e0701b",
    "blockHash": "0x13ff95ae61d6da161cc0c9493199a655ff3e25acce2babdc447efccbf09909c",
    "blockNumber": 0
  }
}
```

Use `snp-report` to decode the `quote` field:

```sh
# Decode the attestation report
./target/debug/snp-report --hex "0x05000000..."

# Or pipe from jq
curl -s -X POST http://localhost:15051 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tee_generateQuote","params":[0,1]}' \
  | jq -r '.result.quote' \
  | ./target/debug/snp-report
```

The output includes:
- **Version**: Report format version (5 = Turin/Genoa)
- **Measurement**: Launch digest to compare against expected value
- **Guest Policy**: Security policy flags (debug, SMT, etc.)
- **TCB Version**: Platform firmware versions
- **Report Data**: User-provided data included in the report
- **Signature**: ECDSA signature for verification

## OVMF Metadata Inspection

Use `ovmf-metadata` to inspect the OVMF firmware's SEV metadata sections:

```sh
./target/debug/ovmf-metadata --ovmf output/qemu/OVMF.fd
```

## Reproducible Builds

Set `SOURCE_DATE_EPOCH` for deterministic output:

```sh
SOURCE_DATE_EPOCH=$(git log -1 --format=%ct) ./build.sh --katana /path/to/katana
```

If unset, `build.sh` warns loudly and falls back to the current wall-clock time
(non-reproducible).

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
source build-config && ./scripts/build-ovmf.sh ./output/qemu
```

Do not use generic OVMF builds from your distribution or other sources when using `kernel-hashes=on` with SEV-SNP.

**Reference:** [AMD's OVMF fork](https://github.com/AMDESE/ovmf) (branch `snp-latest`) contains the SEV-SNP support and hash table memory region required for direct kernel boot with attestation.
