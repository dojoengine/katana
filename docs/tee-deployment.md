# Running a Katana appchain with TEE settlement

This guide walks through standing up a Katana appchain that settles to
Starknet via a TEE attestor. Settlement is **embedded in the katana node**:
the same process that sequences blocks also attests them, proves the
attestations (mock or real SP1), and submits state updates to the Piltover
core contract on the settlement chain. There is no separate prover binary to
operate.

The settlement chain in this guide is Starknet Sepolia, but the same flow works
against any Starknet network — swap the RPC URL and chain ID.

> ## Two deployment modes
>
> | Mode | TEE | Prover | When to use |
> | --- | --- | --- | --- |
> | **Development** | `--tee mock` (no enclave) | Mock proofs (forced by the mock attester) | Local iteration, CI, integration tests. **Never use for production — the mock attester and mock verifier accept anything.** |
> | **Production** | `--tee sev-snp` running inside an AMD SEV-SNP confidential VM | Real SP1 Groth16 proofs via the prover network | Mainnet, testnet with real users, anywhere quotes must be verifiable. |
>
> Pick a mode up front — the bootstrap step deploys different settlement
> contracts for each, and the runtime config differs. The two are not
> interchangeable after the chain is initialized.

## Architecture

```
   ┌───────────────────────────────────────┐    update_state    ┌─────────────────┐
   │                katana                 │ ─────────────────► │ Piltover core   │
   │  (appchain sequencer + settlement)    │                    │ (settlement L2) │
   │                                       │                    └─────────────────┘
   │  blocks ─► attest ─► prove ─► settle  │                            ▲
   └───────────────────────────────────────┘                            │ verifies SP1 proof
        :6969                                                           │
                                                                 ┌──────┴───────┐
                                                                 │ TEE registry │
                                                                 └──────────────┘
```

The Piltover core and TEE registry contracts live on the settlement chain.
Piltover's `validate_input` verifies each state update's SP1 proof through the
registry (its `fact_registry`), recomputes the attestation's Poseidon
commitment from the submitted state transition, and checks the environment
binding (`katana_tee_config_hash`) before advancing the settled state.

Piltover's on-chain state is also the settlement service's only progress
cursor: on (re)start the node reads `get_state()` and resumes from the block
after the last settled one. There is no separate settlement database.

The `tee_generateQuote` / `tee_getEventProof` RPC methods remain available for
**external verifiers** — the embedded service uses the same attestation code
path internally, so an externally fetched quote is byte-identical to what the
node settles with.

## Prerequisites

- `katana` — this repo (`cargo build --release -p katana`).

A funded settlement-chain account:

- **Deployer** — declares + deploys the Piltover core and TEE registry, and
  pays for state-update transactions while running. Needs enough STRK to cover
  declares, deploys, and one settlement tx per batch.

```bash
export SEPOLIA_DEPLOYER_PRIVATE_KEY=0x…
```

**Production additionally requires:**

- An SP1 prover-network key (the settlement service submits Groth16 proving
  jobs to the network).
- An AMD EPYC host with SEV-SNP enabled in BIOS and a host kernel built with
  SEV-SNP support. See [`docs/amdsev.md`](./amdsev.md) for the architecture
  and [`misc/AMDSEV/README.md`](../misc/AMDSEV/README.md) for hardware
  bring-up.
- QEMU 10.2.0 (older versions lack required SEV-SNP features) — build via
  `misc/AMDSEV/build-qemu.sh`.
- The full TEE boot artifact set (OVMF, vmlinuz, initrd, katana binary)
  produced by `misc/AMDSEV/build.sh`.

---

## Development mode (mock TEE)

> **⚠️ Development only.** `--tee mock` skips the enclave entirely and forces
> mock proofs that the Piltover core's mock verifier accepts unconditionally.
> Anyone can produce a valid-looking state update. Do not point this at
> mainnet or any chain with real value at stake.

### 1. Bootstrap (one-time)

#### Deploy the mock TEE registry

The permissive mock registry class (`piltover_mock_amd_tee_registry`) is built
into `katana-contracts` (`crates/contracts/build/`). Declare and deploy it on
the settlement chain with your preferred tooling (e.g. `starkli declare` +
`starkli deploy`; it has no constructor arguments). Note its address — call
this `TEE_REGISTRY_ADDRESS`.

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
`ProgramInfo` to the `KatanaTee` variant (with the chain's
`katana_tee_config_hash`), points its `fact_registry` at the TEE registry, and
writes the rollup's chain config to disk. **Commit `chain-config/` to your
repo** — the genesis keypair is generated on first run and any Dojo manifests
/ profile TOMLs derived from it won't match if you re-init.

#### Configure the settlement runtime

Add a `[settlement.runtime]` section to `chain-config/config.toml`. This is
what enables the embedded settlement service. `init rollup` writes only the
settlement *layer* (under `[settlement.layer.starknet]`); `runtime` is the
operator-local half you add by hand:

```toml
[settlement.runtime]
account-address = "<DEPLOYER_ADDRESS>"
account-private-key = "<SEPOLIA_DEPLOYER_PRIVATE_KEY>"
tee-registry = "<TEE_REGISTRY_ADDRESS>"
batch-size = 1          # blocks per settlement tx; raise for prod
idle-flush-secs = 30    # settle a partial batch after this many idle seconds
# prover-key is omitted: with a mock attester no SP1 proving happens.
```

> **Note:** the settlement account's private key lives in the chain config
> file in plaintext — the file is operator-local, not part of what you publish
> to chain participants. Other nodes following the chain simply omit the
> `[settlement.runtime]` section from their copy.

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
| `--tee mock` | Run as a TEE rollup without a real enclave. **Dev only.** Also forces the settlement service into mock-proof mode. |
| `--dev --dev.no-fee` | Dev mode + free transactions. |
| `--invoke-max-steps` / `--validate-max-steps` | Raised so large Dojo `declare` transactions fit. |
| `--explorer` | Serves the bundled explorer UI on the HTTP port. |

That's it — with the `[settlement-runtime]` section present, the node starts
the settlement service alongside the sequencer. Watch for
`Settlement service started.` and per-batch `Settled block range.` log lines
(target: `settlement`).

---

## Production mode (AMD SEV-SNP)

The production path runs Katana — sequencer and settlement service together —
inside an AMD SEV-SNP confidential VM. With `--tee sev-snp` the settlement
service generates real SP1 Groth16 proofs of the hardware attestations via
the SP1 prover network.

The full architecture, threat model, sealed-storage design, and reproducible
build pipeline are documented in [`docs/amdsev.md`](./amdsev.md). This section
covers only the orchestration around it.

### 1. Build the TEE VM artifacts

On the SEV-SNP host:

```bash
./misc/AMDSEV/build.sh
```

Outputs `OVMF.fd`, `vmlinuz`, `initrd.img`, `katana`, and `build-info.txt` to
`misc/AMDSEV/output/qemu/`. The build is reproducible — set
`SOURCE_DATE_EPOCH` explicitly to get byte-identical artifacts across
machines, which is what makes the launch measurement verifiable by third
parties. See `misc/AMDSEV/build-config` for the pinned package versions and
SHA256s.

For mainnet, use the reproducible Docker-pinned variant:

```bash
./scripts/build-reproducible-katana.sh
./misc/AMDSEV/build.sh --katana ./target/x86_64-unknown-linux-gnu/performance/katana
```

This is the variant `release.yml` produces.

### 2. Bootstrap (one-time)

The bootstrap differs from dev in two places:

- Use the **real TEE registry**, not the mock. The real registry verifies an
  attestation's certificate chain (AMD KDS-rooted VCEK) inside the SP1 proof
  before accepting it. The mock registry skips this check.
- The registry's trusted certificate set must cover your host's processor
  model — the settlement service looks up the trusted prefix length there
  when building each proof.

```bash
katana init rollup \
    --id MY_APPCHAIN \
    --settlement-chain               "$SEPOLIA_RPC_URL" \
    --settlement-account-address     "$DEPLOYER_ADDRESS" \
    --settlement-account-private-key "$SEPOLIA_DEPLOYER_PRIVATE_KEY" \
    --tee \
    --tee-registry-address "$TEE_REGISTRY_ADDRESS" \
    --output-path ./chain-config
```

Then add the settlement runtime to `chain-config/config.toml` — production
additionally needs the SP1 prover key and a larger batch:

```toml
[settlement-runtime]
account-address = "<DEPLOYER_ADDRESS>"
account-private-key = "<SEPOLIA_DEPLOYER_PRIVATE_KEY>"
tee-registry = "<TEE_REGISTRY_ADDRESS>"
prover-key = "<SP1_PROVER_NETWORK_KEY>"
batch-size = 32         # amortize settlement gas; tune to throughput
idle-flush-secs = 60
```

Commit `chain-config/` (minus the `[settlement-runtime]` secrets, if you
publish the config to chain participants) and `build-info.txt` to your repo.
Both are required inputs for any third party reproducing the measurement.

### 3. Run katana inside the SEV-SNP VM

Use `start-vm.sh` — it wires OVMF + kernel + initrd + the disk + the
virtio-serial control channel together and invokes QEMU with the correct
`-object sev-snp-guest,…,kernel-hashes=on` flags.

```bash
sudo ./misc/AMDSEV/start-vm.sh \
    --katana-args "--chain,/mnt/data/chain-config,--http.addr,0.0.0.0,--http.port,5050,--tee,sev-snp,--http.cors-origins,*"
```

The script:

- Boots the measured guest with `-cpu EPYC-v4` and SEV-SNP enabled.
- Keeps the kernel cmdline stable (`console=ttyS0`) so the measurement is
  deterministic.
- Sends `start <args>` over the virtio-serial control channel to launch
  Katana after boot.
- Forwards Katana's port `5050` to host port `15051`.
- Gives the guest 4G of RAM by default (override via `KATANA_MEMORY`). The
  initramfs — including the cairo-native katana binary — unpacks into guest
  RAM, so several GB are required; `-m` is not part of the launch
  measurement, so sizing it differently doesn't change attestation.

The embedded katana is the cairo-native release build: adding
`--enable-native-compilation` to `--katana-args` turns on native execution
of contract classes inside the enclave (off by default).

Notes:

- `--tee sev-snp` (not `--tee mock`) tells Katana to use the real attestation
  path: it asks `/dev/sev-guest` for a `SNP_GET_REPORT` whose `report_data`
  commits to the current state roots — and switches the settlement service to
  real SP1 proving.
- The settlement service runs inside the VM with the node. Its key material
  (`account-private-key`, `prover-key`) therefore lives inside the measured,
  encrypted guest.
- For sealed storage, pass `KATANA_EXPECTED_LUKS_UUID=<uuid>` in the kernel
  cmdline so the init script unseals `/dev/sda` via `SNP_GET_DERIVED_KEY`.
  This changes the launch measurement — verifiers must pin the sealed
  variant. See [`docs/amdsev.md`](./amdsev.md#sealed-storage).
- `--dev` and `--dev.no-fee` **must not** be passed in production.

The Piltover core verifies (a) the SP1 Groth16 proof of the SEV-SNP
attestation (including the AMD KDS certificate chain), (b) that the
`report_data` Poseidon commitment matches the submitted state update, and
(c) the `katana_tee_config_hash` environment binding. If any of these fails,
the settlement transaction reverts — and the node retries with backoff while
continuing to produce blocks.

### 4. Verifier checklist

Anyone consuming state updates from your chain needs to independently:

1. **Reproduce the launch measurement** from OVMF + vmlinuz + initrd + cmdline
   using `snp-digest`. Compare against the measurement attested in the quotes.
2. **Pin a genesis or fork anchor** out-of-band so a freshly-provisioned VM
   can't fake a clean history from block 0.
3. **Walk an unbroken chain of quotes** from that anchor — each quote's
   `prev_block_hash` must match the previous quote's `block_hash`. Quotes are
   served by the node's `tee_generateQuote` RPC.

The full verifier obligations and known residual gaps (whole-disk rollback,
upgrade story) are in [`docs/amdsev.md`](./amdsev.md#trust-model).

---

## Production checklist

Before pointing any of this at mainnet:

- [ ] **TEE mode**: `--tee sev-snp` (not `mock`), inside an SEV-SNP VM.
- [ ] **Settlement runtime**: `prover-key` set (real SP1 proving); registry is
      the real TEE registry (not the mock).
- [ ] **Katana flags**: drop `--dev`, `--dev.no-fee`.
- [ ] **Measurement**: reproducible by third parties from `build-info.txt`.
- [ ] **Sealed storage**: `KATANA_EXPECTED_LUKS_UUID` set in the measured
      cmdline so DB tampering between restarts is rejected.
- [ ] **Build**: `SOURCE_DATE_EPOCH` set; ideally use
      `scripts/build-reproducible-katana.sh`.
- [ ] **Batch size**: raised to amortize settlement gas.
- [ ] **Accounts**: settlement account monitored for STRK balance.
- [ ] **Durable storage**: `chain-config/` and `katana-data/` backed up.
      Losing `chain-config/` means a new genesis. Settlement progress lives
      on-chain in Piltover — there is no separate settlement DB to back up.
- [ ] **Supervision**: the node under systemd/k8s — it does not self-restart.
- [ ] **Network surface**: `--http.cors-origins` and `--http.addr` restricted
      to what you actually need to expose.

## Troubleshooting

**Node exits at startup with a settlement config error** — the
`[settlement-runtime]` section requires `proof_kind = "tee"` on the Starknet
settlement layer, a `--tee` attester, and (for `sev-snp`) a `prover-key`. The
error message names the missing piece.

**`Failed to settle block range; will retry` repeats** — settlement is
failing but the chain keeps producing blocks. Check the error: a revert
mentioning the config hash means the chain spec's chain id / fee token doesn't
match what was pinned at bootstrap; an account error usually means the
settlement account is out of STRK. The service retries the same batch with
backoff and re-reads Piltover's cursor before each retry, so it resumes
cleanly once the cause is fixed. Proving happens at most once per block
range: when the failure is in *submission* (e.g. fees), the service retries
only the `update_state` with the same proof — look for `Failed to submit
state update; will retry with the same proof.` — so a submit-blocked loop
does not keep paying the SP1 prover network. The proof's network reference
is also persisted, so even a node restart recovers the proof from the prover
network (`Recovered proof from the proving network.`) instead of re-proving;
recovery falls back to fresh proving if the network no longer retains it. A
payload the settlement chain rejects in execution (the `update_state`
reverts) is dropped automatically so the next attempt proves fresh.

**`katana init rollup` fails partway through** — the settlement-chain
transactions are not idempotent. Inspect the partial `chain-config/`; usually
fastest to delete it and re-run from a clean state.

**Production `luksOpen` fails on a previously-working disk** — typically a
measurement drift: a kernel, initrd, or OVMF rebuild produced a different
launch measurement, which derives a different LUKS unlock key. The current
policy is "resync from peers after a measurement upgrade." See
[`docs/amdsev.md`](./amdsev.md#trust-model).
