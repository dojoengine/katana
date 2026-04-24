# AMD SEV-SNP TEE VM

Katana can run inside an AMD SEV-SNP confidential virtual machine. The VM's hardware-backed launch measurement binds the Katana binary (and everything else loaded at boot) to a signature the chip produces, which verifiers can check against the AMD Key Distribution Service (KDS). On top of that launch measurement, the `tee_generateQuote` RPC method emits attestation reports whose `report_data` field commits to Katana's current state roots — so a verifier can answer both "is this really the expected Katana binary running on a real AMD chip?" and "which state has it produced?"

Key properties:

- **Confidential compute** -- guest memory is encrypted with a per-VM key the host cannot read, even with root access to the hypervisor.
- **Hardware-backed launch measurement** -- OVMF + kernel + initrd + kernel cmdline are hashed into `AttestationReport.measurement`, signed by the chip's VCEK, verifiable against AMD KDS.
- **State-binding attestation** -- `tee_generateQuote` produces a report whose `report_data` is a Poseidon commitment to block roots (see [`katana-tee`](../crates/tee/src/lib.rs) for the full commitment formula and trust model).
- **Reproducible VM image** -- every input (package versions, SHA256, `SOURCE_DATE_EPOCH`, container image digests) is pinned. Two builds from the same inputs produce byte-identical OVMF/kernel/initrd and therefore the same measurement.
- **Sealed persistent storage (optional)** -- MDBX database lives on a LUKS2 + dm-integrity volume whose key is derived inside the guest via `SNP_GET_DERIVED_KEY`, so the disk only decrypts on the exact chip with the exact measured image.

For how to actually build and run the VM, see [`misc/AMDSEV/README.md`](../misc/AMDSEV/README.md). This document describes the *architecture* — how the pieces fit together and what each one is responsible for.

## Architecture

```mermaid
flowchart TB
    subgraph Host["Host (untrusted)"]
        qemu["qemu-system-x86_64<br/>+ KVM + -object sev-snp-guest"]
        ovmf["OVMF.fd<br/>(AMD SEV fork)"]
        vmlinuz["vmlinuz<br/>(Ubuntu kernel)"]
        initrd["initrd.img<br/>(busybox + katana<br/>+ cryptsetup + modules)"]
        disk["/dev/sda<br/>(LUKS2 or plain ext4)"]
        ctl["/tmp/...control.sock<br/>(virtio-serial)"]
    end

    subgraph Guest["SEV-SNP Guest (measured)"]
        init["init (BusyBox sh)<br/>• parse cmdline vars<br/>• load tsm/sev-guest/dm-*<br/>• unseal_and_mount<br/>• control-channel loop"]
        snpkey["snp-derivekey<br/>(SNP_GET_DERIVED_KEY)"]
        crypt["cryptsetup<br/>(LUKS2 unlock)"]
        mnt["/mnt/data/katana-db<br/>(MDBX)"]
        katana["katana sequencer<br/>(--tee sev-snp)"]
    end

    subgraph Chip["AMD EPYC (hardware root of trust)"]
        psp["Secure Processor (PSP)"]
        vcek["VCEK (per-chip key)"]
    end

    ovmf -->|measured at launch| qemu
    vmlinuz -->|measured at launch| qemu
    initrd -->|measured at launch| qemu
    qemu -->|KVM ioctls<br/>+ SNP_LAUNCH_*| psp
    qemu -.-|virtio-scsi| disk
    qemu -.-|virtio-serial| ctl
    init --> snpkey
    snpkey -->|ioctl| vcek
    snpkey -->|32 bytes via FIFO| crypt
    crypt -->|luksOpen /dev/sda| mnt
    katana -->|tee_generateQuote| psp
    psp -->|VCEK-signed report| katana
    init -->|strip_db_args| katana
    ctl -.->|"start <args>"| init
```

Three layers of trust, outside-in:

1. **Host (untrusted).** Runs QEMU with KVM, attaches the virtual disk, and drives the control channel. The host operator can observe and modify *everything outside the measured boot components and encrypted guest memory.* In particular, the host can swap the contents of `/dev/sda` between VM restarts — which is the whole reason sealed storage exists.
2. **Measured guest (trusted for what's measured).** Everything that gets hashed into the launch measurement: OVMF, kernel, initrd, and the kernel command line. A verifier who reproduces this measurement and matches it against the attestation report knows the guest booted *exactly this code*.
3. **Chip (hardware root of trust).** The AMD Secure Processor derives per-VM memory-encryption keys and per-chip attestation signing keys (VCEK). The launch measurement lives inside the signed attestation report; the VCEK chains up to an AMD root key that's published via KDS.

## Components

| Component | Location | Role |
|-----------|----------|------|
| **OVMF firmware** (`OVMF.fd`) | `misc/AMDSEV/build-ovmf.sh` → `output/qemu/OVMF.fd` | UEFI firmware. Built from [AMD's OVMF fork](https://github.com/AMDESE/ovmf) (`AmdSevX64.dsc`) which reserves the 1 KB hash-table region SEV-SNP uses to inject kernel/initrd/cmdline digests into the measurement. |
| **Linux kernel** (`vmlinuz`) | `misc/AMDSEV/build-kernel.sh` → `output/qemu/vmlinuz` | Ubuntu-supplied kernel; SEV-SNP guest-side drivers (`tsm`, `sev-guest`) plus `dm-mod`, `dm-crypt`, `dm-integrity` are loaded from modules shipped inside the initrd. |
| **initrd** (`initrd.img`) | `misc/AMDSEV/build-initrd.sh` → `output/qemu/initrd.img` | Self-contained root filesystem. Measured at launch. Contains the `init` script and every userspace binary the guest ever runs. See [initrd layout](#initrd-layout) below. |
| **katana binary** | `scripts/build-musl.sh` → `target/x86_64-unknown-linux-musl/release/katana` | Sequencer itself. Statically linked against musl libc so it runs in the minimal initrd without shared libraries. |
| **`snp-derivekey`** (new) | `crates/tee/src/bin/derivekey.rs` (built with `--features snp`) | Tiny helper the init script runs once at boot to derive the LUKS unseal key from `SNP_GET_DERIVED_KEY`. See [Sealed storage](#sealed-storage). |
| **`cryptsetup`** (new) | Built from pinned source inside a pinned Alpine container (SECTION 3 of `build-initrd.sh`) | Drives LUKS2 open/format and dm-integrity setup. Static-musl, single binary, no runtime library dependencies. |
| **`start-vm.sh`** | `misc/AMDSEV/start-vm.sh` | Host-side launcher. Wires OVMF + kernel + initrd + disk + virtio-serial control channel together and invokes QEMU with the correct `-object sev-snp-guest,…,kernel-hashes=on` flags. |
| **`snp-tools/snp-digest`** | `misc/AMDSEV/snp-tools/` | Reproduces the expected launch measurement from the same inputs QEMU hashes. Verifiers use this. |
| **`snp-tools/snp-report`** | `misc/AMDSEV/snp-tools/` | Decodes a raw attestation report into human-readable form. |

### initrd layout

The initrd is a gzipped cpio archive with a single-user layout. It is tiny (~20-30 MB uncompressed) and contains no libc or shared libraries — every binary is statically linked.

```
/
├── bin/
│   ├── busybox            (static, provides /bin/sh and ~20 applets via symlinks)
│   ├── sh → busybox
│   ├── mount → busybox    (and tr, grep, blkid, mkfifo, mkfs.ext2, etc.)
│   ├── katana             (static musl)
│   ├── snp-derivekey      (static; calls SNP_GET_DERIVED_KEY)
│   └── cryptsetup         (static; LUKS2 + dm-integrity driver)
├── lib/modules/
│   ├── tsm.ko             (TSM configfs interface for attestation reports)
│   ├── sev-guest.ko       (/dev/sev-guest device for SNP ioctls)
│   ├── dm-mod.ko          (device-mapper core)
│   ├── dm-crypt.ko        (block-level encryption)
│   └── dm-integrity.ko    (sector-level authentication)
├── init                   (BusyBox shell script; PID 1)
├── dev/, proc/, sys/, tmp/, etc/, mnt/
```

Everything in `bin/` and `lib/modules/` is hashed into the launch measurement (as part of the cpio archive), so any tampering with these files produces a different measurement and verifiers reject the quote.

## Build pipeline

Driven by `misc/AMDSEV/build.sh`, which orchestrates four sub-scripts. The build is *reproducible*: identical inputs produce identical OVMF/kernel/initrd bytes, and therefore the same `AttestationReport.measurement`.

```mermaid
flowchart LR
    src["source + pinned configs<br/>(build-config: versions + SHA256)"]
    sde["SOURCE_DATE_EPOCH"]
    subgraph Build["build.sh"]
        qemu_b["build-qemu.sh<br/>(QEMU 10.2 from source)"]
        ovmf_b["build-ovmf.sh<br/>(AMD OVMF fork)"]
        kernel_b["build-kernel.sh<br/>(Ubuntu vmlinuz extract)"]
        katana_b["scripts/build-musl.sh<br/>(static katana)"]
        initrd_b["build-initrd.sh<br/>(cpio assembly)"]
    end
    out["output/qemu/<br/>{OVMF.fd, vmlinuz,<br/>initrd.img, katana,<br/>build-info.txt}"]

    src --> qemu_b
    src --> ovmf_b
    src --> kernel_b
    src --> katana_b
    sde --> initrd_b
    katana_b --> initrd_b
    kernel_b -->|modules| initrd_b
    ovmf_b --> out
    kernel_b --> out
    initrd_b --> out
```

The `build-initrd.sh` step is the most dependency-heavy:

1. Download pinned `.deb` packages via `apt-get download`: `busybox-static`, `linux-modules`, `linux-modules-extra`. Verify SHA256.
2. Extract them into a staging directory with `dpkg-deb -x`.
3. **Build a statically-linked `cryptsetup`** inside a pinned Alpine container (`CRYPTSETUP_BUILDER_IMAGE=alpine@sha256:…`). Alpine's musl + `*-static` packages (openssl, popt, argon2, …) produce a single binary with no runtime library dependencies, verified via `ldd`.
4. Assemble the initrd directory: install busybox + symlinks, install `cryptsetup`, cherry-pick `tsm.ko` / `sev-guest.ko` / `dm-mod.ko` / `dm-crypt.ko` / `dm-integrity.ko`, copy the Katana binary, copy `snp-derivekey`, emit the `init` script.
5. Normalize timestamps to `SOURCE_DATE_EPOCH`, create a sorted reproducible cpio archive, gzip with `-n`.

## Launch measurement

When QEMU starts the guest with `-object sev-snp-guest,…,kernel-hashes=on`, the AMD Secure Processor (PSP) computes `LAUNCH_MEASURE` over the guest's initial memory. With `kernel-hashes=on`, QEMU additionally injects SHA-256 digests of the kernel, initrd, and kernel command line into a reserved 1 KB region in OVMF, so the measurement covers *all four inputs*.

**What IS in the measurement:**

| Input | Where it's measured |
|-------|---------------------|
| `OVMF.fd` | Loaded into guest memory page-by-page; each page is hashed. |
| `vmlinuz` | SHA-256 injected into OVMF's reserved hash region by QEMU. |
| `initrd.img` | Same as kernel. |
| Kernel cmdline string | Same as kernel. Every byte matters — including the sealed-storage tokens `KATANA_EXPECTED_LUKS_UUID=<uuid>` and `KATANA_ALLOW_FORMAT=1` if present. |

**What is NOT in the measurement** (this is load-bearing):

- The MDBX database on `/dev/sda`. This is exactly why [sealed storage](#sealed-storage) exists.
- Any data fetched over the network at runtime.
- Arguments sent to Katana over the virtio-serial control channel (they arrive *after* launch).
- The data disk's filesystem contents in general.

A verifier reproduces the expected measurement with `snp-digest`:

```sh
snp-digest --ovmf output/qemu/OVMF.fd \
           --kernel output/qemu/vmlinuz \
           --initrd output/qemu/initrd.img \
           --append "console=ttyS0 KATANA_EXPECTED_LUKS_UUID=<uuid>" \
           --vcpus 1 --cpu epyc-v4 --vmm qemu --guest-features 0x1
```

The output digest must equal the `measurement` field of the attestation report. Because the cmdline is measured, sealed-mode and provisioning-mode boots produce *distinct* measurements — verifiers pin only the normal-mode measurement.

## Guest runtime

```mermaid
flowchart TD
    Start([PID 1: /init])
    Mount[Mount /proc /sys /dev /tmp<br/>Create /dev/null /dev/console etc.<br/>Redirect fds to /dev/console]
    LoadSev["insmod tsm.ko sev-guest.ko<br/>Create /dev/sev-guest if needed"]
    Parse["parse_cmdline_vars<br/>(read /proc/cmdline)"]
    LoadDM["load_dm_modules<br/>(dm-mod → dm-crypt → dm-integrity)"]
    Network[Configure eth0<br/>(QEMU user-mode defaults)]
    Decide{SEALED_MODE<br/>= 1?}
    UnsealFlow["unseal_and_mount<br/>• /dev/sev-guest present?<br/>• snp-derivekey → FIFO → cryptsetup<br/>• provisioning: luksFormat<br/>• verify disk UUID = expected<br/>• luksOpen → mkfs.ext2 if empty<br/>• mount /dev/mapper/katana-data"]
    PlainMount["mount -t ext4 /dev/sda /mnt/data<br/>(legacy / dev-only path)"]
    WaitCtl[Wait for org.katana.control.0<br/>virtio-serial port]
    Loop[Read control commands]
    StartCmd["start <csv>"]
    StripArgs["strip_db_args<br/>(drops --db-dir / --db-*)"]
    Katana["/bin/katana --db-dir=/mnt/data/katana-db …"]
    FatalBoot{fatal_boot<br/>on any failure}
    Teardown["teardown_and_halt<br/>• SIGTERM katana (30s grace → KILL)<br/>• umount /mnt/data<br/>• cryptsetup luksClose<br/>• umount /proc /sys etc.<br/>• poweroff -f"]

    Start --> Mount --> LoadSev --> Parse --> LoadDM --> Network --> Decide
    Decide -->|yes| UnsealFlow
    Decide -->|no| PlainMount
    UnsealFlow --> WaitCtl
    PlainMount --> WaitCtl
    WaitCtl --> Loop
    Loop --> StartCmd --> StripArgs --> Katana
    Loop -.->|SIGTERM / SIGINT| Teardown
    FatalBoot -.->|any error| Teardown
```

Key invariants the init script enforces:

- **Failure is always terminal.** Any error — missing `/dev/sev-guest`, UUID mismatch, `luksOpen` failure, mount failure — goes through `teardown_and_halt` and powers off. There is no "continue on best effort" path; a half-unlocked or wrong-UUID disk must never be mountable.
- **Teardown is idempotent.** `fatal_boot` may fire *before* the LUKS device is opened or the mount exists. Every `umount` / `luksClose` tolerates missing state. The `SHUTTING_DOWN` re-entry guard prevents recursion when an error happens inside `teardown_and_halt` itself.
- **The control channel is untrusted.** Arguments arriving over virtio-serial (from the host operator) are *not* measured. `strip_db_args` drops any `--db-dir` / `--db-*` before invoking Katana so the operator cannot redirect Katana out of the sealed mount. The measured initrd owns the `--db-dir` value, not the control channel.
- **Logs go to stderr.** Both stdout and stderr are redirected to `/dev/console`, but `log()` writes only to stderr so `$(strip_db_args …)` captures only the function's real output. Important because several helpers now rely on command substitution.

## Sealed storage

When `KATANA_EXPECTED_LUKS_UUID=<uuid>` is set in the measured kernel cmdline, the init script treats `/dev/sda` as a LUKS2-encrypted volume whose master key is derived from `SNP_GET_DERIVED_KEY`. The derivation inputs are selected so that:

- **Different chip** (different VCEK) → different derived key → `luksOpen` fails.
- **Different measured image** (kernel / initrd / OVMF / cmdline differs) → different derived key → `luksOpen` fails.
- **Tampered ciphertext** (operator modifies sectors out-of-band) → dm-integrity authentication fails at the block layer.

### Key derivation

`snp-derivekey` (`crates/tee/src/bin/derivekey.rs`) issues `SNP_GET_DERIVED_KEY` with:

| Field | Value | Why |
|-------|-------|-----|
| `root_key_select` | `0` (VCEK) | Per-chip identity. The only choice upstream Linux's `/dev/sev-guest` UAPI exposes today. |
| `guest_field_select` | `MEASUREMENT \| GUEST_POLICY` (binary `001001` = 9) | Binds the key to the measured image and guest policy. |
| `guest_svn` | `0` | **Deliberately off.** Mixing SVN would rotate the key on every SVN bump. |
| `tcb_version` | `0` | **Deliberately off.** Same reason: firmware updates would brick the sealed disk. |
| `vmpl` | `1` | Domain separator for the sealed-storage use case. *Not* a privilege boundary — a VMPL0 caller can request a VMPL1 key. |

The binary wraps the 32-byte result in `zeroize::Zeroizing` so the bytes are zeroed from its own stack before exit. A panic hook aborts the process (`SIGABRT`) rather than unwinding, so the key cannot survive in memory through arbitrary destructors.

### LUKS2 + dm-integrity

The init script pipes the 32 bytes from `snp-derivekey` into `cryptsetup` via a 0600-mode FIFO (`/tmp/katana-luks.key`). The FIFO is created fresh per invocation, and the writer exits as soon as `cryptsetup` consumes the key.

LUKS header parameters (set by provisioning boot, checked by every normal boot):

| Parameter | Value | Rationale |
|-----------|-------|-----------|
| `--type` | `luks2` | LUKS1 doesn't support integrity. |
| `--cipher` | `aes-xts-plain64` | Standard block-device cipher. XTS gives confidentiality only, which is why the integrity layer below is non-optional. |
| `--key-size` | `512` | 256-bit AES per half (XTS consumes 2× key length). |
| `--integrity` | `hmac-sha256` | Sector-level authentication via `dm-integrity`. Catches offline ciphertext tampering. |
| `--uuid` | `$KATANA_EXPECTED_LUKS_UUID` | **Pinned** at provisioning time; init-boot refuses to open any other UUID. This is the core of the "no, you can't just wipe the header and re-provision" guarantee. |
| `--pbkdf` | `pbkdf2` | We supply the raw 32-byte key; `argon2id`'s KDF strengthening is wasted compute. |
| `--pbkdf-force-iterations` | `1000` | Minimum allowed. Same reasoning. |

**Why this catches sector-rollback but not whole-disk-rollback.** `dm-integrity` tags each sector with an HMAC whose integrity metadata lives inline on the disk. An attacker who overwrites one sector without the HMAC tag will cause the next read to fail. But an attacker who rolls back *everything* — data + tags + LUKS header — to an earlier snapshot gets a self-consistent disk that opens cleanly. Closing this requires an external monotonic counter, which is out of scope here and tracked as a verifier-side concern. See [trust model](#trust-model).

### Provisioning vs normal boot

```mermaid
flowchart TD
    Start([init: SEALED_MODE=1])
    HasHeader{isLuks<br/>/dev/sda?}
    Fail1[teardown_and_halt<br/>header missing + ALLOW_FORMAT not set<br/>→ header wiped?]
    CanFormat{ALLOW_FORMAT<br/>= 1?}
    Format[luksFormat /dev/sda<br/>--uuid EXPECTED --integrity hmac-sha256]
    UUIDCheck{luksUUID<br/>= EXPECTED?}
    Fail2[teardown_and_halt<br/>disk UUID mismatch<br/>→ disk swapped?]
    Open{luksOpen<br/>succeeds?}
    Fail3[teardown_and_halt<br/>chip mismatch or<br/>measurement drift]
    HasFS{blkid<br/>/dev/mapper/…?}
    Mkfs[mkfs.ext2 on mapper]
    Fail4[teardown_and_halt<br/>no fs + ALLOW_FORMAT not set]
    Mount[mount /dev/mapper/katana-data<br/>at /mnt/data]

    Start --> HasHeader
    HasHeader -->|no| CanFormat
    CanFormat -->|no| Fail1
    CanFormat -->|yes| Format
    Format --> UUIDCheck
    HasHeader -->|yes| UUIDCheck
    UUIDCheck -->|no| Fail2
    UUIDCheck -->|yes| Open
    Open -->|no| Fail3
    Open -->|yes| HasFS
    HasFS -->|yes| Mount
    HasFS -->|no, ALLOW_FORMAT=0| Fail4
    HasFS -->|no, ALLOW_FORMAT=1| Mkfs
    Mkfs --> Mount
```

Two operator-facing modes:

- **Provisioning boot** — operator generates a UUID with `uuidgen`, starts the VM with `start-vm.sh --luks-uuid <uuid> --allow-format`. The cmdline includes `KATANA_ALLOW_FORMAT=1` *which changes the measurement.* The guest luksFormats the disk, mkfs's the mapper, and mounts. The VM then halts (or is halted) and is re-booted in normal mode.
- **Normal boot** — same `--luks-uuid <uuid>` but *without* `--allow-format`. Different cmdline, different measurement. The guest refuses to format or open any disk whose UUID isn't the pinned one. This is the measurement verifiers pin.

Because provisioning and normal boot produce different measurements, an attacker cannot trick a normal boot into formatting. And because the UUID is part of the cmdline (and therefore the measurement), an attacker cannot swap in a different LUKS disk.

## Attestation surface

Once the guest is fully up, Katana listens on the RPC endpoint (forwarded by `start-vm.sh` to `localhost:15051` by default). The `tee_generateQuote` method wraps the SEV-SNP attestation flow:

1. Compute a Poseidon commitment over `(prev_state_root, state_root, prev_block_hash, block_hash, …)`.
2. Copy the 32-byte digest into `report_data[0..32]` (the rest is zero).
3. Issue `SNP_GET_REPORT` via `/dev/sev-guest`.
4. Return the raw 1184-byte report plus metadata.

The exact commitment formula and all the caveats about what it does and doesn't prove live in [`crates/tee/src/lib.rs`](../crates/tee/src/lib.rs) and [`crates/rpc/rpc-api/src/tee.rs`](../crates/rpc/rpc-api/src/tee.rs). That documentation is load-bearing — read it before integrating a verifier.

## Trust model

A TEE quote binds exactly two things: the launch measurement, and the 64-byte `report_data` supplied by the caller. Everything else is outside the guarantee.

- ✅ **The quote proves:** the reported roots were computed by code matching `measurement`, running on a chip whose VCEK chains up to AMD's root key.
- ❌ **The quote does not prove:** that the roots belong to a canonical chain. The guest reads them out of local storage; if the sealed disk is bypassed somehow (e.g. unsealed-mode boot), any roots fit the signature. Sealed mode closes the "operator swaps DB between restarts" hole but not anchor substitution or whole-disk rollback.

Verifier obligations, at minimum:

1. **Reproduce the measurement** from OVMF + vmlinuz + initrd + cmdline and match it against the report. Reject anything else.
2. **Pin the genesis or fork anchor** out-of-band (chain-spec hash published separately, or an L1 contract) so a freshly-provisioned sealed VM can't fake a clean history from block 0.
3. **Walk an unbroken chain of quotes** from that anchor to the block of interest, checking that each quote's `prev_block_hash` matches the previous quote's `block_hash`. The chain forces a tamper to forge every quote, not just one.

Known residual gaps (tracked for follow-up):

- **Whole-disk rollback** within the same VM identity. Needs an external monotonic commitment (e.g. L1 event log).
- **Genesis / fork anchor pinning** is a verifier-side concern; there is no in-repo reference implementation yet.
- **Measurement upgrade story.** Any kernel/initrd/katana upgrade rotates the derived key and renders the old sealed disk unreadable. The current policy is "resync from peers after upgrade." A resealing handshake is a future improvement.

## Related documents

- [`misc/AMDSEV/README.md`](../misc/AMDSEV/README.md) — operational how-to (building, running, decoding reports, troubleshooting).
- [`crates/tee/src/lib.rs`](../crates/tee/src/lib.rs) — `katana-tee` crate docs. Canonical trust-model reference.
- [`crates/rpc/rpc-api/src/tee.rs`](../crates/rpc/rpc-api/src/tee.rs) — `tee_generateQuote` / `tee_getEventProof` RPC schema.
- [`misc/AMDSEV/snp-tools/`](../misc/AMDSEV/snp-tools/) — `snp-digest`, `snp-report`, `ovmf-metadata`.
- [QEMU SEV-SNP launch reference](https://www.qemu.org/docs/master/system/i386/amd-memory-encryption.html#launching-sev-snp)
- [AMD SEV-SNP ABI specification](https://www.amd.com/content/dam/amd/en/documents/developer/56860.pdf)
