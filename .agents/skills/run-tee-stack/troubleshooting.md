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

### `saya-ops` is missing a subcommand or flag the bundle expects

The published `ghcr.io/dojoengine/saya:vX.Y.Z` images may lag behind the saya features the bundle uses (currently: TEE-registry mock subcommand, `--output json`, idempotent deploy via dojo#3404, Piltover v1 `ProgramInfo` alignment, v1 `report_data` mock-prove fix). When that happens we build from source.

**Fix:** the compose builds saya from source via `docker/Dockerfile.saya`, pinned to a specific `SAYA_REV` (currently saya `main` HEAD). When saya cuts a release that includes everything the bundle needs, drop the Dockerfile and have the compose pull the published `ghcr.io/dojoengine/saya:vX.Y.Z` image directly.

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

### `katana init rollup --tee` complains about a missing argument

Passing `--tee` requires `--tee-registry-address`. The compose's `init-chain.sh` reads that address from `/shared/addresses.env` (written by `deploy-contracts.sh` after the mock TEE registry is on chain) and passes it on. If you see this error, `deploy-contracts.sh` either failed silently or the bootstrap volume was wiped between services — check the deploy-contracts logs first.

For the non-TEE custom-URL flow (`--settlement-chain <url>` without `--tee`), `--settlement-facts-registry` is required separately. Not what this bundle uses.

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
