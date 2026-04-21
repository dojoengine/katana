---
name: run-tee-stack
description: How to run the Katana + saya-tee settlement stack. Covers the local mock-prove Docker Compose bundle and provisioning AMD SEV-SNP capable cloud VMs (AWS, Azure, GCP) for real attestation.
---

# Run the Katana TEE stack

Spin up an L3 Katana rollup with TEE attestation, backed by Saya settling blocks onto a Starknet L2 via Piltover.

Two modes:

| Mode | TEE provider | Where it runs | When to use |
|------|--------------|---------------|-------------|
| **Mock-prove** | `katana-tee/tee-mock`, stubbed SP1 | Any Docker host (laptop, cloud, CI) | Integration testing, dev loops, game correctness verification. No hardware required. |
| **Real SEV-SNP** | `katana-tee/snp` + `/dev/sev-guest` | AMD SEV-SNP capable VM (AWS `m6a`/`c6a`, Azure DCasv5, GCP n2d-confidential) | Staging, production, attestation-critical flows. |

Both modes use the same overall topology:

```
  User's L2 (sepolia, local katana --dev, whatever JSON-RPC Starknet)
                      ▲
                      │    piltover + tee_registry live here
                      │    saya-tee submits update_state here
                      │
  ┌───────────────────┴────────────────────────────────────────┐
  │ docker compose                                             │
  │                                                            │
  │  [saya-ops]         one-shot: declare + deploy contracts   │
  │        │            writes /shared/addresses.env, exits    │
  │        ▼                                                   │
  │  [init-chain]       one-shot: katana init rollup           │
  │        │            writes /root/.config/katana/chains/... │
  │        ▼                                                   │
  │  [katana]  (L3)     tee.provider=mock|sev-snp              │
  │        │            exposes RPC on :5050                   │
  │        ▼                                                   │
  │  [saya-tee]         polls L3, proves, settles on L2        │
  │                                                            │
  └────────────────────────────────────────────────────────────┘
```

## Decision tree

1. **Do you need real attestation guarantees?**
   - No → use **mock-prove** locally. Stop here. Jump to [Local mock-prove](#local-mock-prove-quickstart).
   - Yes → continue.
2. **Do you have an SEV-SNP capable host already?**
   - Yes → skip provisioning, jump to [Real SEV-SNP on an existing host](#real-sev-snp-on-an-existing-host).
   - No → use one of the cloud provisioning scripts.
3. **Pick a cloud:**
   - AWS → [aws/provision.sh](aws/provision.sh). Instance family: `m6a`, `c6a`. AMI: Ubuntu 24.04 (`ami-*` varies by region).
   - Azure → [azure/provision.sh](azure/provision.sh). SKUs: `DCasv5`, `ECasv5`, `DCadsv5`, `ECadsv5` (Confidential VMs with SNP).
   - GCP → [gcp/provision.sh](gcp/provision.sh). Family: `n2d-standard-*` with `--confidential-compute-type=SEV_SNP`.

## Local mock-prove quickstart

You provide: an L2 JSON-RPC endpoint + a funded account on that L2.

Fastest path is `katana --dev` as L2 — zero external accounts, runs from cargo-installed katana or a release binary.

```bash
# --- Terminal 1: L2 ---
katana --dev --http.addr 0.0.0.0 --http.port 5051
# Copy one of the PREFUNDED ACCOUNTS entries (Account address + Private key).

# --- Terminal 2: bundle config ---
cd <katana repo root>
cp docker/tee-mock.env.example .env
# Edit .env:
#   SETTLEMENT_RPC_URL=http://host.docker.internal:5051
#   SETTLEMENT_ACCOUNT_ADDRESS=<from Terminal 1>
#   SETTLEMENT_ACCOUNT_PRIVATE_KEY=<from Terminal 1>
#   SETTLEMENT_CHAIN_ID=KATANA    # what `katana --dev` reports by default

docker compose -f docker/tee-mock.compose.yml --env-file .env up --build
```

Wait for:
- `deploy-contracts`: `TEE registry mock address: 0x...` and `Core contract address: 0x...`, then `exit 0`.
- `init-chain`: `Chain spec written to /root/.config/katana/chains/katana_tee_mock`, then `exit 0`.
- `katana`: `RPC server started` on `0.0.0.0:5050`.
- `saya-tee`: `Chain advanced` / `TEE proving completed` on each L3 block.

Drive L3 transactions at `http://localhost:5050`. L3 is in provable mode — it only produces blocks when a tx arrives.

Full details: [docker/README.md](../../../docker/README.md).

## Real SEV-SNP on an existing host

Preconditions on the host:
- AMD SEV-SNP enabled in BIOS/hypervisor
- `/dev/sev-guest` exists and is accessible to the container (`ls -l /dev/sev-guest` returns a char device)
- Docker installed, compose v2 available

There is not yet a `docker/tee.compose.yml` for the real path. **See the follow-up section [v2: real SEV-SNP compose variant](#v2-real-sev-snp-compose-variant).** Until that ships, the adaptation from mock-prove to real is:

1. Build the katana image with `--features tee-snp` instead of `--features tee-mock`.
2. Build saya-tee WITHOUT `--mock-prove` at runtime.
3. Deploy the real SP1 fact registry contract on L2 (not the TEE registry mock).
4. Mount `/dev/sev-guest` into the katana container with `devices: - /dev/sev-guest`.
5. Provide a real prover network account private key to saya-tee.

Each of these has an open upstream dependency. See the [v2 follow-up](#v2-real-sev-snp-compose-variant).

## Cloud provisioning scripts

Each script spins up one SEV-SNP capable VM, installs Docker, clones the katana repo, and boots the compose stack. All three are idempotent — re-running updates in place rather than creating duplicates.

- **[aws/provision.sh](aws/provision.sh)** — EC2 `m6a.large` in the region of your choice. Uses an Ubuntu 24.04 LTS AMI lookup, creates a VPC + security group + key pair if missing, tags resources with `katana-tee-stack` for easy teardown via `aws/cleanup.sh`.
- **[azure/provision.sh](azure/provision.sh)** — Confidential VM on `Standard_DC2as_v5`. Creates resource group, vnet, NSG with port 5050 open.
- **[gcp/provision.sh](gcp/provision.sh)** — `n2d-standard-2` with `--confidential-compute-type=SEV_SNP`. Requires project with Confidential VMs API enabled.

Each script prompts for required inputs (region, instance size, SSH key path). Read the script before running — they mutate cloud infrastructure.

### What every provisioning script does

```
1. Verify cloud CLI is installed + authenticated
2. Create network primitives (VPC/vnet, subnet, SG/NSG with port 22 + 5050)
3. Provision the SEV-SNP capable VM with Ubuntu 24.04 LTS
4. SSH in, install docker + docker-compose
5. git clone this repo at a pinned SHA
6. Write .env with the user's L2 creds
7. docker compose up --build
8. Wait for health, print the public RPC endpoint
```

## Troubleshooting

See [troubleshooting.md](troubleshooting.md) for the full list. Quick reference:

| Symptom | Likely cause | Fix |
|---------|--------------|-----|
| `deploy-contracts` exits 2 with `declare-and-deploy-tee-registry-mock` not found | Image pulled is older saya release without the subcommand | Compose now builds saya from source via `docker/Dockerfile.saya` pinned to rev `5a3b8c9`. Pull latest of this repo. |
| Build fails `no space left on device` | `.dockerignore` missing entries | `.claude/`, `target/`, `katana-test-db/` must be excluded. Verify root `.dockerignore`. |
| `init-chain` errors on RPC connect | L2 not reachable from the compose network | On native Linux Docker, ensure `extra_hosts: host.docker.internal:host-gateway` is set (already in the compose). Check L2 is bound to `0.0.0.0`, not `127.0.0.1`. |
| `saya-tee` can't sign txs on L2 | `SETTLEMENT_CHAIN_ID` mismatch | `katana --dev` reports chain id `KATANA` (`0x4b4154414e41`), not `SN_SEPOLIA`. Match what the L2 actually returns. |
| `scarb not found on PATH` during saya build | Docker RUN steps don't inherit `$HOME/.local/bin` | `Dockerfile.saya` explicitly `ENV PATH=/root/.local/bin:$PATH` before the cargo build. |

## v2: real SEV-SNP compose variant

Planned as a follow-up PR. Three upstream blockers to resolve first:

1. **Real fact_registry contract on L2.** Mock mode uses the TEE registry mock as piltover's `fact_registry_address` — piltover's `verify_sp1_proof` becomes a passthrough. Real mode needs an actual SP1 verifier deployed. Contract identity + deployment story are not yet documented by the piltover team.
2. **Prover network account provisioning.** `saya-tee` in real mode requires `--prover-private-key` for the SP1 prover network. No sign-up flow is documented.
3. **Published katana image with `--features tee-snp` only.** Today, the `ghcr.io/dojoengine/katana` default build already enables `tee-snp` via `katana-cli/default`. For a real TEE mode compose, the existing image is actually sufficient — but a smoke test on an SEV-SNP host is needed to confirm.

Once those resolve, expected deliverables:

- `docker/tee.compose.yml` — like `tee-mock.compose.yml` but:
  - Uses `ghcr.io/dojoengine/katana:<tag>` (no new image needed)
  - `--tee.provider=sev-snp` instead of `mock`
  - `devices: [ "/dev/sev-guest" ]` on the katana service
  - Real fact_registry address in `saya-ops setup-program`
  - `saya-tee` service without `--mock-prove`, with real prover key
- `docker/tee.env.example` — real L2 + real prover creds
- `docker/scripts/deploy-contracts-real.sh` — deploys real fact_registry (not mock TEE registry)

## Source of truth pointers

- Docker bundle: [docker/](../../../docker/)
- Compose file: [docker/tee-mock.compose.yml](../../../docker/tee-mock.compose.yml)
- Saya build: [docker/Dockerfile.saya](../../../docker/Dockerfile.saya) pinned to saya rev `5a3b8c9`
- Katana tee features: `crates/tee/Cargo.toml` (`snp`, `tee-mock`), `crates/cli/Cargo.toml` (`tee`, `tee-snp`, `tee-mock`), `bin/katana/Cargo.toml` (passthroughs)
- Integration test (in-process reference implementation): `tests/saya-tee/`
