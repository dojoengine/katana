# Running a Katana appchain with TEE settlement

This guide walks through standing up the two long-running services that make a
Katana appchain settle to Starknet via a TEE attestor:

1. **`katana`** — the appchain sequencer. Produces blocks locally.
2. **`saya-tee`** — the prover/attestor. Takes blocks off the appchain, proves
   them (mock or real), and submits state updates to the Piltover core contract
   on the settlement chain.

The settlement chain in this guide is Starknet Sepolia, but the same flow works
against any Starknet network — swap the RPC URL and chain ID.

> ## Two deployment modes
>
> | Mode | TEE | Prover | When to use |
> | --- | --- | --- | --- |
> | **Development** | `--tee mock` (no enclave) | `saya-tee --mock-prove` (mock proofs) | Local iteration, CI, integration tests. **Never use for production — the mock attestor and mock verifier accept anything.** |
> | **Production** | `--tee sev-snp` running inside an AMD SEV-SNP confidential VM | Real Stone/SHARP prover | Mainnet, testnet with real users, anywhere quotes must be verifiable. |
>
> Pick a mode up front — the bootstrap step deploys different settlement
> contracts for each, and the runtime flags differ. The two are not
> interchangeable after the chain is initialized.

## Architecture

```
   ┌─────────────┐     blocks      ┌──────────────┐    state update    ┌─────────────────┐
   │   katana    │ ──────────────► │   saya-tee   │ ─────────────────► │ Piltover core   │
   │ (appchain)  │                 │  (attestor)  │                    │ (settlement L2) │
   └─────────────┘                 └──────────────┘                    └─────────────────┘
        :6969                                                          ▲
                                                                       │ verifies attestor
                                                                       │
                                                                ┌──────┴───────┐
                                                                │ TEE registry │
                                                                └──────────────┘
```

The Piltover core and TEE registry contracts live on the settlement chain. The
registry holds the set of attestors that the core will accept state updates
from; `saya-tee` registers itself there at startup.

## Prerequisites

Binaries on `PATH`:

- `katana` — this repo (`cargo build --release -p katana`).
- `saya-tee` and `saya-ops` — from [`cartridge-gg/saya`][saya]. Supported
  version: `0.4.0`.
- `jq` — for parsing JSON from `saya-ops`.

A funded settlement-chain account:

- **Deployer** — declares + deploys the Piltover core and TEE registry, and
  pays for state-update transactions while running. Needs enough STRK to cover
  declares, deploys, and one settlement tx per batch.
- **Prover** — signs the attestor registration in the TEE registry. Can be the
  same account as the deployer in development. For production, use a separate
  key controlled by the operator of the SEV-SNP machine.

Export the private keys before running anything:

```bash
export SEPOLIA_DEPLOYER_PRIVATE_KEY=0x…
export SEPOLIA_PROVER_PRIVATE_KEY=0x…
```

**Production additionally requires:**

- An AMD EPYC host with SEV-SNP enabled in BIOS and a host kernel built with
  SEV-SNP support. See [`docs/amdsev.md`](./amdsev.md) for the architecture
  and the [dojoengine/katana-tee-vm](https://github.com/dojoengine/katana-tee-vm) README for hardware
  bring-up.
- QEMU 10.2.0 (older versions lack required SEV-SNP features) — build via
  `scripts/build-qemu.sh` in [katana-tee-vm](https://github.com/dojoengine/katana-tee-vm).
- The full TEE boot artifact set (OVMF, vmlinuz, initrd, katana binary)
  produced by [katana-tee-vm](https://github.com/dojoengine/katana-tee-vm)'s `build.sh`.

[saya]: https://github.com/cartridge-gg/saya

---

## Development mode (mock TEE)

> **⚠️ Development only.** `--tee mock` skips the enclave entirely and
> `--mock-prove` submits mock proofs that the Piltover core's mock verifier
> accepts unconditionally. Anyone can produce a valid-looking state update.
> Do not point this at mainnet or any chain with real value at stake.

### 1. Bootstrap (one-time)

#### Deploy the mock TEE registry

```bash
saya-ops core-contract \
    --account-address  "$DEPLOYER_ADDRESS" \
    --private-key      "$SEPOLIA_DEPLOYER_PRIVATE_KEY" \
    --settlement-rpc-url "$SEPOLIA_RPC_URL" \
    --settlement-chain-id sepolia \
    --output json \
    declare-and-deploy-tee-registry-mock \
    --salt 0x…
```

Pick a stable `--salt` so the address is deterministic across re-runs. Grab
`contract_address` from the JSON output — call this `TEE_REGISTRY_ADDRESS`.

#### Initialize the rollup

```bash
katana init rollup \
    --id MY_APPCHAIN_DEV \
    --settlement-chain               "$SEPOLIA_RPC_URL" \
    --settlement-account-address     "$DEPLOYER_ADDRESS" \
    --settlement-account-private-key "$SEPOLIA_DEPLOYER_PRIVATE_KEY" \
    --tee \
    --tee-registry-address "$TEE_REGISTRY_ADDRESS" \
    --output-path ./chain-config
```

This declares and deploys the Piltover core on the settlement chain, wires its
`ProgramInfo` to the Katana TEE program hash, and writes the rollup's chain
config to disk. **Commit `chain-config/` to your repo** — the genesis keypair
is generated on first run and any Dojo manifests / profile TOMLs derived from
it won't match if you re-init.

Read the Piltover address back out for the saya step:

```bash
PILTOVER_ADDRESS=$(grep '^address' chain-config/config.toml | head -1 | awk -F'"' '{print $2}')
```

### 2. Run katana (dev)

```bash
mkdir -p ./katana-data

katana \
    --chain ./chain-config \
    --http.addr 0.0.0.0 \
    --http.port 6969 \
    --http.cors-origins "*" \
    --explorer \
    --tee mock \
    --dev --dev.no-fee \
    --data-dir ./katana-data \
    --invoke-max-steps   100000000 \
    --validate-max-steps  10000000
```

| flag | why |
| --- | --- |
| `--tee mock` | Run as a TEE rollup without a real enclave. **Dev only.** |
| `--dev --dev.no-fee` | Dev mode + free transactions. |
| `--invoke-max-steps` / `--validate-max-steps` | Raised so large Dojo `declare` transactions fit. |
| `--explorer` | Serves the bundled explorer UI on the HTTP port. |

### 3. Run saya-tee (dev)

```bash
mkdir -p ./saya-data

saya-tee tee start \
    --mock-prove \
    --rollup-rpc                     "http://localhost:6969" \
    --settlement-rpc                 "$SEPOLIA_RPC_URL" \
    --settlement-piltover-address    "$PILTOVER_ADDRESS" \
    --settlement-account-address     "$DEPLOYER_ADDRESS" \
    --settlement-account-private-key "$SEPOLIA_DEPLOYER_PRIVATE_KEY" \
    --tee-registry-address           "$TEE_REGISTRY_ADDRESS" \
    --prover-private-key             "$SEPOLIA_PROVER_PRIVATE_KEY" \
    --db-dir ./saya-data \
    --batch-size 1 \
    --attestor-poll-interval-ms 1000 \
    --idle-timeout-secs 30
```

| flag | why |
| --- | --- |
| `--mock-prove` | Submit mock proofs the mock verifier accepts. **Dev only.** |
| `--batch-size 1` | One block per settlement tx. Dev-friendly; raise for prod. |
| `--idle-timeout-secs` | Flushes a partial batch after N idle seconds. |

---

## Production mode (AMD SEV-SNP)

The production path runs Katana inside an AMD SEV-SNP confidential VM and
points `saya-tee` at the VM's forwarded RPC port. Saya disables `--mock-prove`
so it submits real proofs.

The full architecture, threat model, sealed-storage design, and reproducible
build pipeline are documented in [`docs/amdsev.md`](./amdsev.md). This section
covers only the orchestration around it.

### 1. Build the TEE VM artifacts

On the SEV-SNP host, the build scripts live in the dedicated
[dojoengine/katana-tee-vm](https://github.com/dojoengine/katana-tee-vm) repository:

```bash
git clone https://github.com/dojoengine/katana-tee-vm
cd katana-tee-vm
./build.sh --katana /path/to/katana
```

Outputs `OVMF.fd`, `vmlinuz`, `initrd.img`, `katana`, and `build-info.txt` to
`output/qemu/`. The build is reproducible — set
`SOURCE_DATE_EPOCH` explicitly to get byte-identical artifacts across
machines, which is what makes the launch measurement verifiable by third
parties. See katana-tee-vm's `build-config` for the pinned package versions and
SHA256s.

For mainnet, use the reproducible Docker-pinned variant:

```bash
# In the katana repo:
./scripts/build-reproducible-katana.sh
# Then in the katana-tee-vm checkout:
./build.sh --katana <katana-repo>/target/x86_64-unknown-linux-gnu/performance/katana
```

This is the variant `release.yml` produces.

### 2. Bootstrap (one-time)

The bootstrap differs from dev in two places:

- Use the **real TEE registry**, not the mock. The real registry verifies an
  attestor's launch measurement against the AMD KDS-rooted VCEK signature
  before accepting it. The mock registry skips this check.
- Pin the **expected launch measurement** in the Piltover core's
  `ProgramInfo`. `katana init rollup` reads the measurement from the build
  outputs and writes it into the deployment.

```bash
# Deploy the real TEE registry. See `saya-ops core-contract --help` for the
# exact subcommand — varies by saya-ops version.
saya-ops core-contract \
    --account-address  "$DEPLOYER_ADDRESS" \
    --private-key      "$SEPOLIA_DEPLOYER_PRIVATE_KEY" \
    --settlement-rpc-url "$SEPOLIA_RPC_URL" \
    --settlement-chain-id sepolia \
    --output json \
    declare-and-deploy-tee-registry \
    --salt 0x…

# Compute the launch measurement for the artifacts you just built
# (snp-digest is built from the snp-tools crate in katana-tee-vm).
snp-digest \
    --ovmf   output/qemu/OVMF.fd \
    --kernel output/qemu/vmlinuz \
    --initrd output/qemu/initrd.img \
    --append "console=ttyS0" \
    --vcpus 1 --cpu epyc-v4 --vmm qemu --guest-features 0x1

# Initialize with --tee sev-snp (not just --tee). Pass the measurement and
# the real registry address.
katana init rollup \
    --id MY_APPCHAIN \
    --settlement-chain               "$SEPOLIA_RPC_URL" \
    --settlement-account-address     "$DEPLOYER_ADDRESS" \
    --settlement-account-private-key "$SEPOLIA_DEPLOYER_PRIVATE_KEY" \
    --tee \
    --tee-registry-address "$TEE_REGISTRY_ADDRESS" \
    --output-path ./chain-config
```

Commit `chain-config/` and `build-info.txt` to your repo. Both are required
inputs for any third party reproducing the measurement.

### 3. Run katana inside the SEV-SNP VM

Use `start-vm.sh` — it wires OVMF + kernel + initrd + the disk + the
virtio-serial control channel together and invokes QEMU with the correct
`-object sev-snp-guest,…,kernel-hashes=on` flags.

```bash
# From the katana-tee-vm checkout:
sudo ./start-vm.sh \
    --katana-args "--chain,/mnt/data/chain-config,--http.addr,0.0.0.0,--http.port,5050,--tee,sev-snp,--http.cors-origins,*"
```

The script:

- Boots the measured guest with `-cpu EPYC-v4` and SEV-SNP enabled.
- Keeps the kernel cmdline stable (`console=ttyS0`) so the measurement is
  deterministic.
- Sends `start <args>` over the virtio-serial control channel to launch
  Katana after boot.
- Forwards Katana's port `5050` to host port `15051`.

Notes:

- `--tee sev-snp` (not `--tee mock`) tells Katana to use the real
  `tee_generateQuote` path: it asks `/dev/sev-guest` for a `SNP_GET_REPORT`
  whose `report_data` commits to the current state roots.
- For sealed storage, pass `KATANA_EXPECTED_LUKS_UUID=<uuid>` in the kernel
  cmdline so the init script unseals `/dev/sda` via `SNP_GET_DERIVED_KEY`.
  This changes the launch measurement — verifiers must pin the sealed
  variant. See [`docs/amdsev.md`](./amdsev.md#sealed-storage).
- `--dev` and `--dev.no-fee` **must not** be passed in production.

### 4. Run saya-tee (production)

Run `saya-tee` on a separate machine (or at minimum a separate process) from
the SEV-SNP host. Point it at the host port the VM forwards Katana on.

```bash
mkdir -p ./saya-data

saya-tee tee start \
    --rollup-rpc                     "http://<sev-snp-host>:15051" \
    --settlement-rpc                 "$SEPOLIA_RPC_URL" \
    --settlement-piltover-address    "$PILTOVER_ADDRESS" \
    --settlement-account-address     "$DEPLOYER_ADDRESS" \
    --settlement-account-private-key "$SEPOLIA_DEPLOYER_PRIVATE_KEY" \
    --tee-registry-address           "$TEE_REGISTRY_ADDRESS" \
    --prover-private-key             "$SEPOLIA_PROVER_PRIVATE_KEY" \
    --db-dir ./saya-data \
    --batch-size 32 \
    --attestor-poll-interval-ms 1000 \
    --idle-timeout-secs 60
```

Key differences from dev:

- **No `--mock-prove`.** Saya runs the real prover and submits real Stone
  proofs to the Piltover core.
- **Higher `--batch-size`** to amortize settlement gas. Tune to your
  throughput and gas budget.
- The Piltover core verifies (a) the SEV-SNP attestation report against AMD
  KDS, (b) that the report's measurement matches the value pinned at
  bootstrap, (c) that the `report_data` Poseidon commitment matches the
  submitted state update, and (d) the proof itself. If any of these fails,
  the settlement transaction reverts.

### 5. Verifier checklist

Anyone consuming state updates from your chain needs to independently:

1. **Reproduce the launch measurement** from OVMF + vmlinuz + initrd + cmdline
   using `snp-digest`. Compare against the measurement pinned in the
   Piltover core's `ProgramInfo`.
2. **Pin a genesis or fork anchor** out-of-band so a freshly-provisioned VM
   can't fake a clean history from block 0.
3. **Walk an unbroken chain of quotes** from that anchor — each quote's
   `prev_block_hash` must match the previous quote's `block_hash`.

The full verifier obligations and known residual gaps (whole-disk rollback,
upgrade story) are in [`docs/amdsev.md`](./amdsev.md#trust-model).

---

## Production checklist

Before pointing any of this at mainnet:

- [ ] **TEE mode**: `--tee sev-snp` (not `mock`), inside an SEV-SNP VM.
- [ ] **Prover mode**: drop `--mock-prove`.
- [ ] **Katana flags**: drop `--dev`, `--dev.no-fee`.
- [ ] **Registry**: real TEE registry (not the mock).
- [ ] **Measurement**: pinned in `ProgramInfo` at bootstrap; reproducible by
      third parties from `build-info.txt`.
- [ ] **Sealed storage**: `KATANA_EXPECTED_LUKS_UUID` set in the measured
      cmdline so DB tampering between restarts is rejected.
- [ ] **Build**: `SOURCE_DATE_EPOCH` set; ideally use
      `scripts/build-reproducible-katana.sh`.
- [ ] **Batch size**: raised to amortize settlement gas.
- [ ] **Accounts**: separate deployer and prover keys, monitored balances.
- [ ] **Durable storage**: `chain-config/`, `katana-data/`, and `saya-data/`
      backed up. Losing `chain-config/` means a new genesis; losing
      `saya-data/` means re-indexing from genesis (state remains intact).
- [ ] **Supervision**: both services under systemd/k8s — neither
      self-restarts.
- [ ] **Network surface**: `--http.cors-origins` and `--http.addr` restricted
      to what you actually need to expose.

## Troubleshooting

**`saya-tee` exits with "attestor not registered"** — the registry transaction
was rejected. Confirm `--tee-registry-address` matches the contract from the
bootstrap, and that the prover account has STRK. In production, also confirm
the launch measurement the VM reports matches what's pinned in `ProgramInfo`.

**`katana init rollup` fails partway through** — the settlement-chain
transactions are not idempotent. Inspect the partial `chain-config/`; usually
fastest to delete it and re-run from a clean state.

**State updates stop landing on settlement** — check that the deployer
account on settlement still has STRK to pay for the settlement transactions.

**Production `luksOpen` fails on a previously-working disk** — typically a
measurement drift: a kernel, initrd, or OVMF rebuild produced a different
launch measurement, which derives a different LUKS unlock key. The current
policy is "resync from peers after a measurement upgrade." See
[`docs/amdsev.md`](./amdsev.md#trust-model).
