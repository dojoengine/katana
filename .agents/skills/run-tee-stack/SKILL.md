---
name: run-tee-stack
description: How to run the Katana + saya-tee settlement stack. Covers the local mock-prove Docker Compose bundle, provisioning AMD SEV-SNP capable cloud VMs (AWS, Azure, GCP), and deploying to any SSH-reachable host (bare metal from latitude.sh/Hetzner/OVH, on-prem boxes, or an existing VM the user already owns).
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
2. **Do you have a host already (cloud VM, bare metal, on-prem box)?**
   - Yes, and you just want to deploy on it → [byo-host/provision.sh](byo-host/provision.sh). Works for anything SSH-reachable: latitude.sh, Hetzner, OVH, Vultr bare metal, a leftover EC2, on-prem hardware, etc. Detects `/dev/sev-guest` so you can tell at a glance whether real SEV mode will work on that host.
   - No, spin one up via hyperscaler cloud → continue to step 3.
3. **Pick a cloud:**
   - AWS → [aws/provision.sh](aws/provision.sh). Instance family: `m6a`, `m7a`, `c6a`, `c7a`. AMI: Ubuntu 24.04 (`ami-*` varies by region). Enables SEV-SNP via `CpuOptions.AmdSevSnp=enabled`.
   - Azure → [azure/provision.sh](azure/provision.sh). SKUs: `DCasv5`, `ECasv5`, `DCadsv5`, `ECadsv5` (Confidential VMs). Image must be the `cvm` variant, not stock Ubuntu.
   - GCP → [gcp/provision.sh](gcp/provision.sh). Family: `n2d-standard-*` + `--confidential-compute-type=SEV_SNP` + `--min-cpu-platform="AMD Milan"`.
   - None of the above? → use `byo-host/provision.sh` against whatever shell you do have.

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

## Deploying on a host you already have

**Use this if:** you've got an SSH-reachable box from latitude.sh, Hetzner, OVH, Vultr bare metal, a leftover cloud VM, or an on-prem machine.

**[byo-host/provision.sh](byo-host/provision.sh)** — fully cloud-agnostic. You give it an IP, a user, and an SSH private key; it detects whether `/dev/sev-guest` is present (tells you at a glance whether real SEV mode will work), installs docker, clones this repo, writes your `.env`, and runs compose. Works on any Ubuntu 22.04+/Debian 12+ host with a sudo-capable user.

```bash
HOST_IP=1.2.3.4 \
SSH_USER=ubuntu \
SSH_KEY_PATH=~/.ssh/id_rsa \
SETTLEMENT_RPC_URL=https://... \
SETTLEMENT_ACCOUNT_ADDRESS=0x... \
SETTLEMENT_ACCOUNT_PRIVATE_KEY=0x... \
SETTLEMENT_CHAIN_ID=sepolia \
./byo-host/provision.sh
```

Teardown: `HOST_IP=... SSH_KEY_PATH=... ./byo-host/cleanup.sh`. Optionally `REMOVE_CHECKOUT=1` to also wipe `~/katana` on the host.

### What the script assumes about the host

| Need | Detail |
|------|--------|
| OS | Ubuntu 22.04 / 24.04 or Debian 12+. Other distros may work, not tested. |
| Privileges | Sudo-capable user (passwordless sudo preferred). Root works too. |
| Network | Port 5050 reachable from wherever you'll submit txs. Opening the firewall is your job — the script doesn't manage network ACLs. |
| For real SEV-SNP | `/dev/sev-guest` present. The script reports what it finds so you know up front. |

### Why this exists separately from the cloud-specific scripts

Bare-metal providers (especially latitude.sh, Hetzner) often offer SEV-SNP-enabled AMD EPYC hardware at a fraction of hyperscaler Confidential VM pricing, but they don't have a "confidential compute" API abstraction — you get a box with a shell. That's the target shape of this script: no cloud API, just "I have root on a Linux box with the right CPU." Also works great for homelab / on-prem deployments.

## Cloud provisioning (hyperscalers)

Use these when you want a fresh SEV-SNP capable VM with minimal effort and are willing to pay hyperscaler Confidential Computing prices. Each script spins up one VM, installs Docker, clones katana, and brings up the compose stack. All are idempotent — re-running reuses existing resources.

- **[aws/provision.sh](aws/provision.sh)** — EC2 `m7a.large` in the region of your choice. Uses an Ubuntu 24.04 LTS AMI lookup, creates a security group if missing, tags resources with `katana-tee-stack` for easy teardown via `aws/cleanup.sh`. SEV-SNP enabled via `CpuOptions.AmdSevSnp=enabled`.
- **[azure/provision.sh](azure/provision.sh)** — Confidential VM on `Standard_DC2as_v5`. Creates resource group, vnet, NSG with port 5050 open. Requires the `cvm` image variant — a regular Ubuntu on a DC-series VM will NOT give you an attestation device.
- **[gcp/provision.sh](gcp/provision.sh)** — `n2d-standard-2` with `--confidential-compute-type=SEV_SNP` and `--min-cpu-platform="AMD Milan"`. Requires project with Compute Engine + Confidential Computing APIs enabled.

Read the script before running — they mutate cloud infrastructure.

### What every cloud provisioner does (same shape as byo-host, plus a VM)

```
1. Verify cloud CLI is installed + authenticated
2. Create network primitives (security group / NSG / firewall rule with port 22 + 5050)
3. Provision the SEV-SNP capable VM with Ubuntu 24.04 LTS
4. SSH in and verify /dev/sev-guest actually came up (see "SEV-SNP verification" below)
5. Install docker, clone repo, write .env, run compose
6. Print the public IP + RPC endpoint
```

The post-provision step (installing docker + running compose) is conceptually the same path as `byo-host/provision.sh`. If you're debugging the post-provision stage and want to iterate faster, use `byo-host` against the already-provisioned VM's public IP.

### SEV-SNP verification (REQUIRE_SEV)

Every provisioner sources [`lib/verify-sev.sh`](lib/verify-sev.sh) and runs it after SSH is reachable. The helper SSHs to the host and checks for `/dev/sev-guest`. The cloud provisioners default to `REQUIRE_SEV=1` (fail if absent) because you explicitly asked for SEV-SNP hardware; byo-host defaults to `REQUIRE_SEV=0` (warn, proceed) because byo-host users often target a plain VM for mock-prove testing.

Override from the caller:

```bash
REQUIRE_SEV=0 ./aws/provision.sh       # proceed on a non-SEV AWS instance (e.g. for mock-prove on a cheap m5)
REQUIRE_SEV=1 ./byo-host/provision.sh  # fail loudly if my bare-metal box isn't SEV-capable after all
```

This exists because cloud providers sometimes silently downgrade Confidential VM requests: wrong image variant on Azure, wrong CPU platform on GCP, AMI/instance-family mismatch on AWS. You pay Confidential prices but get regular hardware. The check catches that before the stack runs.

## Real SEV-SNP on an existing host (mock-prove today, real v2 pending)

The host-side preconditions are:
- AMD SEV-SNP enabled in BIOS/hypervisor
- `/dev/sev-guest` exists as a character device
- Docker + compose v2 installed

The cloud + byo-host scripts handle the install + compose-up side. The mode they run today is **mock-prove** (via `docker/tee-mock.compose.yml`), which works on any host regardless of SEV support.

**Real SEV-SNP mode is a v2 follow-up.** See [v2: real SEV-SNP compose variant](#v2-real-sev-snp-compose-variant) for the three open upstream blockers. Once that ships, the same provisioning scripts will switch from `tee-mock.compose.yml` → `tee.compose.yml` and add `/dev/sev-guest` to the katana service's `devices:` list.

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
