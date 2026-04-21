# run-tee-stack troubleshooting

Failures we've actually hit while bringing up the bundle, and what to change.

## Compose / docker build

### `no space left on device` during `Sending build context`

The build context copies the entire repo (minus `.dockerignore` excludes) before the first FROM step. If `.dockerignore` is missing entries, the context balloons past the VM disk.

**Fix:** ensure the top-level `.dockerignore` excludes at minimum:
```
target/
.claude/          # ← includes worktrees with their own target/ dirs
katana-test-db/
.env
node_modules/
```
One katana worktree with a populated `target/` can easily be 40+ GB.

### `curl: not found` silently produces an empty scarb install

The scarb install script is `curl ... | sh`. If curl isn't present, curl fails but the pipe exit code is from `sh` (which exits 0 with empty stdin). The failure surfaces two steps later as "scarb: not found".

**Fix:** in `docker/Dockerfile.saya`, make sure `curl` is in the apt-get install list, and set `SHELL ["/bin/bash", "-eo", "pipefail", "-c"]` at the top of the stage so pipe failures are caught.

### `scarb: not found` despite install succeeding

The scarb installer puts a wrapper at `$HOME/.local/bin/scarb` (symlinked to the real binary under `$HOME/.local/share/scarb-install/<ver>/bin/scarb`). Docker RUN steps don't automatically add `$HOME/.local/bin` to PATH.

**Fix:** `ENV PATH="/root/.local/bin:${PATH}"` in the Dockerfile after the install, before any RUN that calls `scarb`.

### `saya-ops core-contract declare-and-deploy-tee-registry-mock` not found

The published `ghcr.io/dojoengine/saya:latest` image is saya v0.3.1, which predates the tee-registry-mock subcommand (added in saya PR #60 at rev `5a3b8c9`).

**Fix:** the compose builds saya from source via `docker/Dockerfile.saya`, pinned to `SAYA_REV=5a3b8c9`. If you see this error, your local compose is outdated — re-pull the repo.

### `cargo build -p saya-ops` → "package did not match any packages"

Saya's `bin/ops/` and `bin/persistent-tee/` are INDEPENDENT workspaces (each has its own `[workspace]` at the top of their Cargo.toml). They are not members of the root saya workspace, so `cargo build -p saya-ops` from the root fails.

**Fix:** build each bin from its own directory — `(cd bin/ops && cargo build --release)`, `(cd bin/persistent-tee && cargo build --release)`. Matches saya's own `release.yml`.

## Runtime / compose startup

### `init-chain` fails to reach the L2 at `http://host.docker.internal:<port>`

`host.docker.internal` works on Docker Desktop for macOS/Windows. On native Linux Docker, it needs `extra_hosts: host.docker.internal:host-gateway` on the service. Our compose sets this on all services; if you forked and dropped it, restore it.

Also verify the L2 is bound to `0.0.0.0`, not `127.0.0.1` — `host.docker.internal` routes to the host's external interface, so a loopback-only listener is unreachable.

### saya-ops signs txs the L2 rejects: "invalid chain id"

`katana --dev` reports chain id `KATANA` (`0x4b4154414e41`) by default, not `SN_SEPOLIA`.

**Fix:** in `.env`, set `SETTLEMENT_CHAIN_ID=KATANA`. For Sepolia, use `sepolia`. For mainnet, `mainnet`. Match what the L2 actually returns from `starknet_chainId`.

### `katana init rollup` complains when `--settlement-chain` is a URL

The `--settlement-facts-registry` flag is required when a custom URL is passed (the built-in chains sepolia/mainnet have a hardcoded default). The compose's `init-chain.sh` always passes the TEE registry mock address as `--settlement-facts-registry` — correct for TEE-mock mode where the TEE registry mock doubles as piltover's fact registry.

### `saya-tee` logs "Chain advanced" but no settlement txs on L2

The L3 is in provable mode and only emits blocks when a tx arrives. `saya-tee` settles L3 blocks, so no L3 blocks = no settlement txs.

**Fix:** submit any tx to `http://localhost:5050`. A simple self-transfer from a prefunded L3 account will tick one block. The compose sets `BATCH_SIZE=1` by default, so settlement should happen within ~20s.

## Cloud provisioning scripts

### AWS: `UnauthorizedOperation` on RunInstances with CpuOptions

The IAM role/user needs `ec2:RunInstances` plus the newer `ec2:ModifyInstanceCpuOptions` in some regions, and your account needs to be opted in to SEV-SNP capable instance families (usually automatic, but the account age thing).

### Azure: "The requested size for resource is currently not available"

Confidential VM SKUs (DCasv5, DCadsv5, ECasv5, ECadsv5) aren't available in every region. Check:
```bash
az vm list-skus --resource-type virtualMachines --location <region> \
    --query "[?name=='Standard_DC2as_v5'].restrictions" -o json
```
If you see `"reasonCode": "NotAvailableForSubscription"` or `"Location"`, pick a different region (eastus, westus2, westeurope usually have them).

### GCP: `Invalid value for field 'resource.confidentialInstanceConfig.confidentialComputeType': SEV_SNP`

The N2D machine type is required + the minCpuPlatform must be `AMD Milan` or later. Some zones don't have Milan capacity — try `us-central1-a`, `us-east1-b`, `europe-west4-a`.

You also need the Confidential Computing API enabled in the project:
```bash
gcloud services enable compute.googleapis.com confidentialcomputing.googleapis.com
```

## When all else fails

- Tail compose logs per-service: `docker compose -f docker/tee-mock.compose.yml logs -f <service>`
- Run any service interactively: `docker compose -f docker/tee-mock.compose.yml run --rm --entrypoint /bin/sh <service>`
- Nuke everything and start over: `docker compose -f docker/tee-mock.compose.yml down -v && docker system prune -f`
- The in-process reference that proves the topology works: `tests/saya-tee/` in the katana repo. If the compose diverges from what that test does, trust the test.
