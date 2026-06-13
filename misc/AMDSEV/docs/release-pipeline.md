# Release Pipeline

How a `katana-tee-vm` release is produced, what each artifact is, what makes
the output reproducible, and how to operate the pipeline. The implementation
lives in [`.github/workflows/amdsev-release.yml`](../../../.github/workflows/amdsev-release.yml);
this document explains it.

A release bundles everything needed to boot Katana inside an AMD SEV-SNP
confidential VM — `OVMF.fd`, `vmlinuz`, `initrd.img` (with the katana binary
embedded) — together with the **sealed launch measurement** that attestation
verifiers pin against. The measurement is the product; the artifacts exist to
make it reproducible.

## Trigger contract

The workflow runs on three triggers:

| Trigger | Behavior |
|---|---|
| **`release: published`** (primary) | Fires for every katana release. Builds a VM image bundling that release's katana binary, from the release tag's commit, and publishes a `katana-<release tag>` GitHub Release (e.g. `katana-v1.9.0`). Then dispatches the hardware E2E test against it. |
| Push of a tag matching `katana-v*` | Manual **pipeline re-release** against the same katana version. Same full build + publish, keyed on the pushed tag. |
| `workflow_dispatch` | Dry run: identical build and measurement, artifacts uploaded to the workflow run, **no GitHub Release created**. |

Because the primary trigger fires on every katana release, a VM image
follows each katana version automatically — the embedded binary comes from
the release that fired the workflow, and the VM is built from that release's
commit so the `misc/AMDSEV` tooling and the binary are a consistent,
reproducible pair.

**No release→release loop.** The job publishes a `katana-v*` release, which
is itself a `release: published` event. A job-level guard skips any release
whose tag already starts with `katana-v`, and — belt and braces — releases
created with the workflow's `GITHUB_TOKEN` do not re-trigger workflows at
all. The follow-on hardware test (`amdsev-snp-e2e`) is therefore dispatched
explicitly (`workflow_dispatch` *is* permitted from `GITHUB_TOKEN`).

The VM tag and embedded katana version:

| Event | VM release tag | Embedded katana version |
|---|---|---|
| katana release `v1.9.0` published | `katana-v1.9.0` | `v1.9.0` |
| katana release `v1.9.0-rc.1` published | `katana-v1.9.0-rc.1` | `v1.9.0-rc.1` |
| push tag `katana-v1.9.0-pipeline.2` | `katana-v1.9.0-pipeline.2` | `v1.9.0` — the `-pipeline.N` suffix is stripped |

The `-pipeline.N` push form exists for **pipeline-only re-releases**: cutting
a new VM image against the *same* katana version, e.g. after a fix to the
build scripts or a pin bump. Pre-releases propagate — a VM release built from
a katana pre-release is itself marked pre-release.

`workflow_dispatch` takes two inputs:

- `katana_version` — katana release tag to bundle (or `latest`).
- `force_rebuild` — set to `true` to bypass artifact reuse (see below) and
  rebuild OVMF and the kernel from scratch.

The runner is stock `ubuntu-latest`. Nothing in the pipeline needs SEV-SNP
hardware: OVMF is cross-compiled, the kernel comes from a `.deb`, and
`snp-digest` is pure userspace hashing. SNP hardware is only needed at VM-boot
time, which is outside this workflow.

## Pipeline walkthrough

The steps below run in order; each corresponds to a named step in
`amdsev-release.yml`.

### 1. Pin `SOURCE_DATE_EPOCH`

```sh
SOURCE_DATE_EPOCH=$(git log -1 --format=%ct)
```

Every timestamp embedded in build output (OVMF internals, initrd cpio mtimes)
comes from this value. Pinning it to the HEAD commit time makes the build a
pure function of *(commit, katana version, pins)* instead of *(…, wall-clock
time)*. Without it, `build.sh` falls back to `date +%s` with a loud warning
and the resulting measurement is unreproducible by anyone else.

#### How the epoch enters each artifact

| Artifact | Affected? | Mechanism |
|---|---|---|
| `initrd.img` | **Yes** | Every file in the initrd tree gets its mtime set to the epoch (`touch -d @$SOURCE_DATE_EPOCH`, `build-initrd.sh`) before packing, and member mtimes are part of the newc cpio bytes. The gzip wrapper uses `-n` (no embedded timestamp) and `cpio --reproducible` with sorted input handles the rest, so the epoch is the *only* time-derived input to the archive bytes. The static cryptsetup/mkfs.ext2 binaries baked into the initrd are themselves built with the same epoch passed into their pinned Alpine container (`build-cryptsetup.sh`). |
| `OVMF.fd` | **Yes — but from its own pin, not the release epoch** | EDK2's BaseTools read `SOURCE_DATE_EPOCH` from the environment for the timestamps they embed in the firmware. `build-ovmf.sh` deliberately *overrides* the release epoch with the pinned OVMF commit's own timestamp, so `OVMF.fd` is a pure function of (`OVMF_COMMIT`, toolchain): any checkout of any tag rebuilds byte-identical firmware. The value used is recorded as `OVMF_SOURCE_DATE_EPOCH` in `build-info.txt`. |
| `vmlinuz` | **No** | Prebuilt Ubuntu artifact, extracted from the pinned `.deb` — its bytes are whatever Canonical shipped, regardless of epoch. |
| `katana` | **No** | Prebuilt release binary from dojoengine/katana, downloaded as-is. |
| `build-info.txt` | Recorded only | Carries a `SOURCE_DATE_EPOCH=` line so reproducers know which value to use; not a measured artifact. (Its `# Generated:` comment is wall-clock and intentionally outside any verification.) |

Net effect: **the release epoch reaches the measurement only through
`initrd.img`** (OVMF's epoch is derived from its own pin and is stable as
long as `OVMF_COMMIT` is). The same source tree built with the same release
epoch — the value recorded in `build-info.txt` — produces identical artifacts
and an identical measurement; a different release epoch changes the initrd
bytes (and the measurement) even though nothing functional changed. Two
pipeline behaviors follow directly from this:

1. Releases pin the release epoch to the HEAD commit time, so the measurement
   is a function of the tagged commit rather than of when the workflow
   happened to run.
2. From-source reproducers must set `SOURCE_DATE_EPOCH` to the value recorded
   in the release's `build-info.txt`, not to their own checkout time. (OVMF
   needs no such care — `build-ovmf.sh` derives its epoch from the pin
   automatically.)

### 2. Install toolchain

`nasm`, `iasl`, `uuid-dev` (EDK2/OVMF build), `musl-tools` (static
`snp-derivekey`), `zstd`, `cpio` (initrd packaging), plus the
`x86_64-unknown-linux-musl` Rust target. Docker (for the static cryptsetup
container build) is preinstalled on the runner.

### 3. Resolve and download katana

The version resolved from the tag (or dispatch input) is downloaded from
[dojoengine/katana releases](https://github.com/dojoengine/katana/releases)
with the pattern `katana_*_linux_amd64.tar.gz`. Both anchors matter: the
`katana_` prefix excludes the `paymaster-service_*` / `vrf-server_*` tarballs
that newer katana releases ship alongside it, and the anchored suffix excludes
the CPU-tuned `_native` variant — releases embed the **portable** build, which
is the one produced by katana's reproducible-build pipeline.

### 4. Reuse OVMF/kernel from the previous release (when pins match)

Before building, the workflow looks up the most recent published release,
downloads its `build-info-<tag>.txt`, and compares pins against the current
`build-config`:

- **OVMF** is reused when `OVMF_COMMIT` is unchanged *and* the previous
  artifact records an `OVMF_SOURCE_DATE_EPOCH` (i.e. it was built under the
  "epoch = OVMF commit time" rule; older artifacts built with a release
  epoch are rebuilt once rather than perpetuating bytes that no tag checkout
  can reproduce).
- **vmlinuz** is reused when `KERNEL_VERSION` and `KERNEL_PKG_SHA256` are
  unchanged.

A reused artifact is extracted from the previous release's tarball and
**verified against the SHA-256 recorded in that release's `build-info.txt`**
before being accepted. Any mismatch, missing asset, or missing field falls
back to rebuilding that component; `force_rebuild=true` skips reuse entirely.

Because OVMF's epoch is derived from its pin, rebuilding it from the same
pin reproduces the same bytes (given the same toolchain) — so reuse is a
time saver (~10 minutes of EDK2 build) plus a shield against the one
remaining nondeterminism: toolchain drift on `ubuntu-latest` (EDK2 output
depends on the runner's gcc/nasm/iasl versions, which Ubuntu updates under
us). Consecutive releases that only bump katana differ **only in the initrd
hash**, which is what verifiers maintaining pinned measurements want.

Reused components are recorded in the provenance as
`OVMF_REUSED_FROM=<tag>` / `KERNEL_REUSED_FROM=<tag>`.

### 5. Build components (`build.sh`)

`./build.sh --katana <bin> [ovmf] [kernel] initrd` builds only the components
not satisfied by reuse. All version pins and package checksums come from
[`build-config`](../build-config), which is the single source of truth.

- **OVMF** (`scripts/build-ovmf.sh`) — builds `OvmfPkg/AmdSev/AmdSevX64.dsc`
  from [AMD's edk2 fork](https://github.com/AMDESE/ovmf) at the pinned
  `OVMF_COMMIT`. The AmdSev platform is required: it reserves the hash-table
  region that QEMU's `kernel-hashes=on` injects kernel/initrd/cmdline hashes
  into (generic OVMF builds abort at launch with a "firmware hashes table
  area is invalid" error).
- **Kernel** (`scripts/build-kernel.sh`) — downloads the pinned Ubuntu
  `linux-image-unsigned-<KERNEL_VERSION>-generic` `.deb`, verifies
  `KERNEL_PKG_SHA256`, and extracts `vmlinuz`. No compilation.
- **Initrd** (`scripts/build-initrd.sh`) — the heart of the image. Downloads
  and checksum-verifies the pinned busybox, kernel-module, and glibc runtime
  `.deb`s; installs the katana binary plus its exact dynamic runtime (resolved
  via `readelf`, copied only from the pinned packages — never from the build
  host); embeds the init script; and packs a reproducible cpio (sorted file
  order, normalized modes, `SOURCE_DATE_EPOCH` timestamps, `--reproducible`).
  Two helper binaries are built on demand for the sealed-storage flow:
  - static `cryptsetup` + `mkfs.ext2`, compiled from pinned sources inside a
    digest-pinned Alpine container (`scripts/build-cryptsetup.sh`, needs
    Docker);
  - static `snp-derivekey` (musl), from the `snp-tools` crate — it performs
    the `SNP_GET_DERIVED_KEY` ioctl in-guest to unlock the LUKS data disk.

`build.sh` then writes `build-info.txt`. It **merges** with an existing
`build-info.txt`, overwriting only the fields of components it actually built
— this is what lets step 4 seed the file with the previous release's values so
reused components keep accurate provenance.

### 6. Compute the sealed launch measurement

The workflow builds `snp-tools` and runs `snp-digest` over the three boot
artifacts plus the launch configuration:

```sh
snp-digest --ovmf OVMF.fd --kernel vmlinuz --initrd initrd.img \
    --append "$(build_sealed_cmdline "$KATANA_CANONICAL_LUKS_UUID")" \
    --vcpus 1 --cpu epyc-v4 --vmm qemu --guest-features 0x1
```

Key facts:

- The measured cmdline comes from `scripts/sealed-cmdline.sh` — the single
  source of truth shared with `start-vm.sh` and `verify-build.sh`, so all
  three always hash the same bytes.
- The published measurement is bound to `KATANA_CANONICAL_LUKS_UUID` from
  `build-config`. Operators running with their own per-host UUID get a
  *different* measurement and must recompute it themselves (the README's
  [Launch Measurement](../README.md#launch-measurement) section explains what
  the digest covers and why).
- The output is validated to be 96 lowercase hex chars (SHA-384) before being
  accepted, then appended to `build-info.txt` as `LUKS_UUID=` and
  `LAUNCH_MEASUREMENT=` lines.

### 7. Stage, render notes, publish

- `katana-tee-vm-<tag>.tar.gz` is created from the whole output directory;
  `build-info.txt` and `launch-measurement.txt` are also staged as standalone
  assets for one-click access.
- Release notes are rendered from `build-info.txt`: the measurement +
  LUKS_UUID block, per-artifact SHA-256 table, pinned upstream sources
  (OVMF commit, kernel package, katana tag), verification instructions, and
  the full `build-info.txt` embedded in a collapsible section.
- On tag refs only, a GitHub Release is created with those assets and notes.

## Published artifacts

| Asset | Contents |
|---|---|
| `katana-tee-vm-<tag>.tar.gz` | `OVMF.fd`, `vmlinuz`, `initrd.img`, `katana` (the embedded binary, for convenience), `build-info.txt`, `launch-measurement.txt` |
| `build-info-<tag>.txt` | Full provenance: pins, package checksums, artifact SHA-256s, `SOURCE_DATE_EPOCH`, reuse markers, measurement |
| `launch-measurement-<tag>.txt` | The sealed launch measurement, bare hex, one line |

## What moves the measurement between releases

| Change | Measurement effect |
|---|---|
| New katana version | Initrd hash changes → new measurement |
| Any change to `scripts/build-initrd.sh` or the init script | Initrd hash changes → new measurement |
| Bumping any package pin in `build-config` (busybox, glibc, kernel modules…) | Initrd hash changes → new measurement |
| Bumping `OVMF_COMMIT` | OVMF rebuilt → new measurement |
| Bumping `KERNEL_VERSION` | Kernel hash changes → new measurement |
| Changing `KATANA_CANONICAL_LUKS_UUID` | Measured cmdline changes → new measurement |
| Katana-only release with unchanged pins (artifact reuse active) | **Only the initrd hash moves**; OVMF and kernel contributions are byte-identical to the previous release |

Verifiers should treat every release's measurement as new and take it from
`launch-measurement-<tag>.txt`; the table above is for understanding *why* it
moved.

## Verifying a release

Anyone can check a downloaded release end to end:

```sh
mkdir -p /tmp/ktv && tar xzf katana-tee-vm-<tag>.tar.gz -C /tmp/ktv
cargo build -p snp-tools --release   # provides snp-digest
./verify-build.sh /tmp/ktv
```

`verify-build.sh` asserts every artifact SHA-256 against `build-info.txt`,
then recomputes the launch measurement from the artifacts + the recorded
`LUKS_UUID` (via the shared `sealed-cmdline.sh`) and compares it to the
recorded `LAUNCH_MEASUREMENT`. Exit code is non-zero on any mismatch.

Full **from-source reproduction** is one command:

```sh
git fetch --tags && git checkout <tag>
./reproduce-release.sh <tag>
```

`reproduce-release.sh` downloads the release's published `build-info.txt` and
the exact katana binary it embedded (verified against the recorded
`KATANA_BINARY_SHA256`), rebuilds OVMF + kernel + initrd from source with the
recorded `SOURCE_DATE_EPOCH`, and then runs `verify-build.sh` against the
**published** provenance — so exit code 0 means the bytes you built yourself
match the release and hash to the published launch measurement. This works
identically for artifacts the release inherited via reuse (`*_REUSED_FROM`
markers): OVMF derives its own epoch from the pinned commit, so it rebuilds
byte-identically from any tag checkout.

The one caveat is the OVMF toolchain — EDK2 output depends on the gcc/nasm/
iasl versions, so reproduce on the same OS image the release used
(`ubuntu-latest` at build time) for an exact byte match.

## Runbook

**Cut a release for a new katana version** — nothing to do. Publishing the
katana release fires `amdsev-release` automatically: it builds the VM image
against the release commit, publishes `katana-<ver>`, and dispatches the
hardware E2E. Watch the `amdsev-release` run; on success, sanity-check the
release notes and `launch-measurement-<tag>.txt`, then the `amdsev-snp-e2e`
run it dispatched.

**Re-release the same katana version** (build-script fix, pin bump, broken
earlier release): push a `katana-<ver>-pipeline.N` tag (N incrementing) at the
commit you want built — `git tag katana-<ver>-pipeline.N && git push origin
katana-<ver>-pipeline.N`.

**Bump a pin** (kernel, busybox, glibc runtime, OVMF commit, cryptsetup…):
update both the version and its SHA-256 in `build-config`. PR CI (the
`amdsev-initrd-test` workflow) exercises most pins on every PR; OVMF pin bumps are
only exercised by the release workflow itself, so prefer a `workflow_dispatch`
dry run before tagging.

**Dry-run the pipeline** without publishing: `gh workflow run amdsev-release.yml -f
katana_version=<ver>` (optionally `-f force_rebuild=true`). Artifacts land on
the workflow run.

## Relationship to PR CI

The same scripts are exercised continuously outside releases:

- `amdsev-initrd-test` (every PR / main push touching `misc/AMDSEV`) builds the sealed initrd with the same
  `build-initrd.sh` and pins, checks byte-reproducibility by building twice,
  and boot-tests the result in plain QEMU — including the control-channel
  protocol and Katana RPC liveness.
- `amdsev-lint` shellchecks all build scripts plus the guest init (as POSIX sh);
  `amdsev-snp-tools` runs the crate's tests, which pin the key-derivation contract
  that sealed storage depends on.

The release pipeline is therefore mostly "pre-validated": historically, the
only release-time-only risks are the OVMF build (skipped in PR CI for speed)
and pin rot in the Ubuntu archive (caught within a day by CI runs, since the
archive drops superseded package versions).

## Troubleshooting

| Symptom | Likely cause / fix |
|---|---|
| `apt-get download` fails with "Can't find a source to download version …" | Pin rot: Ubuntu's archive only serves the latest version of an updated package. Bump the version + SHA-256 in `build-config` (verify the hash against the signed APT `Packages` index). |
| `docker: … registry-1.docker.io … context deadline exceeded` | Transient Docker Hub failure pulling the pinned Alpine builder. Re-run the job. |
| `SEV: guest firmware hashes table area is invalid` at VM boot | Generic OVMF used instead of the AmdSev platform build — see the README troubleshooting section. |
| Measurement differs from a previous release despite "no changes" | Check `build-info.txt` diffs: a pin bump, an initrd content change, or OVMF rebuilt with a new `SOURCE_DATE_EPOCH` (reuse not active — was `force_rebuild` set, or did `OVMF_COMMIT` change?). |
| Reuse step warns about a SHA-256 mismatch | The previous release's tarball doesn't match its own build-info — investigate before trusting that release; the workflow already fell back to a fresh build. |
