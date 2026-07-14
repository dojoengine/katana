# Installing the Katana TEE VM on an SEV-SNP host

`install.sh` turns a machine with AMD SEV-SNP enabled into a Katana TEE VM
host from a published release, in one command:

```sh
curl -fsSL https://raw.githubusercontent.com/dojoengine/katana/main/misc/AMDSEV/install.sh | bash
```

It preflights the host, downloads and verifies a `tee-vm-v*` release, walks
through an interactive wizard (vCPUs, memory, data disk, ports, storage
sealing), builds the pinned QEMU when the host lacks it, recomputes the
expected launch measurement for the chosen configuration, and generates a
foreground `run.sh`. It deliberately does **not** install a service:
persistence is the operator's choice ([recipes below](#running-and-persistence)).

The installer needs no root; `run.sh` invokes `start-vm.sh` via `sudo`
(KVM and disk setup require it).

## Host prerequisites

- **AMD EPYC (Milan or newer) with SEV-SNP enabled in BIOS** (SME + SNP
  settings), and an SNP-capable host kernel (6.11+, or a distro/vendor SNP
  kernel) with `kvm_amd` loaded with `sev_snp=1`. The preflight checks
  `/dev/sev`, `/sys/module/kvm_amd/parameters/sev_snp`, and `/dev/kvm`, and
  prints remediation hints for whichever is missing.
- **Bare metal.** On clouds this means a bare-metal AMD instance (for
  example AWS `m6a.metal` / `c6a.metal`; equivalent AMD bare-metal offerings
  on GCP/Azure/OVH/Latitude etc.). Regular confidential VMs are SNP *guests*
  and cannot host nested SNP guests.
- **Tools:** `curl tar sha256sum python3 dd mkfs.ext4 socat`
  (`apt-get install -y curl tar coreutils python3 e2fsprogs socat`).
- **QEMU** at the version pinned by the release (currently 10.2.0). If the
  host has another version (or none), the installer offers to build it from
  source via the release's `scripts/build-qemu.sh` — locally under the
  install root (default) or into `/usr/local`.
- **Rust (optional):** measurement verification uses `snp-digest`. Each
  release's `build-config` pins a prebuilt `snp-tools-v*` release (tag +
  SHA-256) that the installer downloads and checksum-verifies automatically,
  so `cargo` is only needed as a fallback — for releases predating the pin,
  or if you prefer building the verifier from source (the prebuilt is a
  convenience copy, not the trust root). Without either, the install still
  completes (artifact checksums are always verified), but attestation
  verification has no local expected value until you run `install.sh verify`
  with one of them available.

## What gets installed

Everything lands under `--home` (default `~/.katana/tee-vm`):

```
~/.katana/tee-vm/
├── config.env                  # all wizard answers, sourceable shell
├── run.sh                      # generated foreground launcher
├── expected-measurement.txt    # launch measurement for THIS host's config
├── install.sh                  # self-copy: upgrades / verify / print-systemd
├── data.img                    # persistent VM data disk (default path)
├── qemu/                       # locally-built QEMU (only if built locally)
├── current -> releases/<tag>
└── releases/<tag>/
    ├── boot/                   # OVMF.fd, vmlinuz, initrd.img, build-info.txt, ...
    └── src/                    # misc/AMDSEV at the tag (start-vm.sh, scripts/, snp-tools/)
```

The release tarball carries only boot artifacts; the launcher scripts are
fetched from the repo source **at the same tag** — they are a matched pair,
and the boot artifacts + launcher of one release must not be mixed with
another's.

## The wizard

Every question has a flag / env-var override, and `--yes` skips the wizard
entirely (missing values fall back to config-file values from a previous
run, then to defaults). Re-runs pre-fill from the existing `config.env`.

| Question | Default | Flag / env | Notes |
|---|---|---|---|
| Release tag | latest `tee-vm-v*` | `--tag` / `KATANA_TEE_TAG` | |
| vCPUs | 1 | `--vcpus` / `KATANA_VCPUS` | **Part of the launch measurement** — the expected measurement is recomputed for your value. Locked to 1 on releases whose launcher predates configurable vCPUs. |
| Memory | 4G | `--memory` / `KATANA_MEMORY` | **Not measured** — size freely. The initramfs (including the katana binary) unpacks into guest RAM; 4G minimum advised. |
| Data disk path | `<home>/data.img` | `--data-disk` / `KATANA_DATA_DISK` | Created if absent; never touched on upgrade. |
| Data disk size (MB) | 1024 | `--disk-size-mb` / `KATANA_DISK_SIZE_MB` | Only asked when the disk doesn't exist yet. |
| Host RPC port | 15051 | `--rpc-port` / `HOST_RPC_PORT` | Forwards to guest port 5050. |
| Storage mode | unsealed | `--sealed` / `--unsealed` | Sealed = LUKS2 + dm-integrity, key derived in-guest and bound to the launch measurement — an upgrade re-keys the disk. See the README's *Storage sealing* section. |
| Katana args | `start-vm.sh` default CSV | `--katana-args` | Unmeasured (delivered via fw_cfg). The `--metrics.port` inside it drives the metrics host forward. |

Non-interactive example (provisioning tools, cloud-init):

```sh
curl -fsSL .../install.sh | bash -s -- --yes --vcpus 4 --memory 8G --rpc-port 15051
```

## Running and persistence

```sh
~/.katana/tee-vm/run.sh
```

runs the VM in the foreground: the `start-vm.sh` banner, then a live tail of
the guest serial log. Ctrl+C triggers a graceful guest shutdown (katana
TERM, sync, unmount, poweroff). The data disk persists, so the next run
resumes the existing chain state.

How (and whether) to keep it running is up to you:

- **systemd** — `~/.katana/tee-vm/install.sh print-systemd` prints a sample
  unit (auto-restart on failure, start on boot, graceful stop). It is only
  printed, never installed; review it, then copy it to
  `/etc/systemd/system/` per the instructions in its header.
- **tmux / screen** — just run `run.sh` inside a session.
- **anything else** — `run.sh` is a plain foreground process that exits
  non-zero on failure; any supervisor can manage it.

Once running:

- RPC: `http://localhost:<rpc-port>` (starknet JSON-RPC + `tee_generateQuote`)
- Metrics: `http://localhost:<metrics-port>` (from `--metrics.port` in the
  katana args; on by default at 9100)

## Verifying attestation

The installer writes `expected-measurement.txt` — the SEV-SNP launch
measurement your configuration should produce (it depends on the boot
artifacts, kernel cmdline variant, and vCPU count; **not** on memory size,
katana args, or disk contents). Compare it against a live quote:

```sh
curl -s -X POST http://localhost:15051 \
    -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","id":1,"method":"tee_generateQuote","params":[null,0]}'
# decode with snp-report (built from releases/<tag>/src/snp-tools); the
# measurement is the 48-byte field at offset 0x90 of the report.
```

`install.sh verify` re-checks the downloaded artifacts against
`build-info.txt`, re-verifies the release's published measurement, and
recomputes `expected-measurement.txt` for the saved configuration.

## Upgrading

Re-run the installer (`~/.katana/tee-vm/install.sh`, or re-curl it). The new
release lands in its own `releases/<tag>/` directory — previous releases are
kept, so rolling back is re-running with `--tag <old-tag>`. The wizard is
pre-filled from `config.env`; the data disk is never touched.

**Sealed mode caveat:** the sealed disk key is bound to the launch
measurement, so changing releases re-keys the disk and the old data no
longer unseals. The installer warns and asks for confirmation (or
`KATANA_CONFIRM_SEALED_UPGRADE=1` non-interactively). Point `--data-disk` at
a fresh file to keep the old disk recoverable under the old release.

## Troubleshooting

- **`/dev/sev not present`** — SNP disabled in BIOS, non-EPYC hardware, or a
  virtualized (non-bare-metal) machine.
- **`kvm_amd sev_snp not enabled`** —
  `echo 'options kvm_amd sev_snp=1 sev=1 sev_es=1' | sudo tee /etc/modprobe.d/kvm-amd-snp.conf`,
  then reload `kvm_amd` (or reboot). The host kernel must support SNP.
- **QEMU build fails** — install the build deps first:
  `sudo apt-get install -y build-essential ninja-build pkg-config libglib2.0-dev libpixman-1-dev python3-venv flex bison wget`.
- **Measurement not computed** — the release pins no prebuilt `snp-digest`
  (its `build-config` predates the pin) and `cargo` is missing. Install Rust
  (https://rustup.rs), then `~/.katana/tee-vm/install.sh verify`.
- **vCPUs locked to 1** — the chosen release's launcher predates
  configurable vCPUs; pick a newer `tee-vm-v*` release.
- **Boot or runtime issues** — see the README's *Troubleshooting* section;
  the serial log path is printed by `run.sh` at startup.
