# Katana TEE mock-prove dev stack

One-command Docker compose bundle that spins up an L3 katana rollup with mock
TEE attestation, deploys the piltover core contract on your L2, and runs
saya-tee to settle L3 blocks onto the L2 via piltover.

No AMD SEV-SNP hardware required. Proofs are stubbed (`--mock-prove`) and
the TEE registry mock doubles as the fact registry, so piltover's on-chain
`verify_sp1_proof` becomes a passthrough. Everything else is real: state-diff
Poseidon commitments, block-by-block settlement, piltover's `validate_input`
round-trip.

## What this is (and isn't)

**Yes:** a fast way for Dojo app/game devs to verify their L3 transactions
flow through the full katana → saya-tee → piltover pipeline end-to-end, using
their own L2 (Sepolia works great).

**No:** production-grade TEE attestation. Real AMD SEV-SNP is a follow-up —
see the v2 issue. Don't use this mode for anything that actually needs
hardware attestation guarantees.

## Requirements

- Docker with compose v2 (`docker compose`, not `docker-compose`)
- An L2 settlement layer you can write to. Two easy paths:
  - **Local `katana --dev`** — fastest, no external accounts, nothing to fund. See [Quickstart](#quickstart-local-katana-l2) below.
  - **Starknet Sepolia** — more realistic. Fund a Starknet account with a bit of STRK from the [faucet](https://starknet-faucet.vercel.app/). Use `SETTLEMENT_RPC_URL=https://starknet-sepolia.public.blastapi.io/rpc/v0_9`.

## Quickstart (local katana L2)

This is the recommended first run. Zero external dependencies, zero
network calls, fully reproducible.

```bash
# --- Terminal 1: start local L2 katana ---
# (Install via `cargo install --path bin/katana` if needed, or use a release binary.)
katana --dev --http.addr 0.0.0.0 --http.port 5051
# Watch the startup log. Copy one of the PREFUNDED ACCOUNTS entries —
# you need the `Account address` and `Private key` fields.

# --- Terminal 2: prep the compose bundle ---
cp docker/tee-mock.env.example .env
# Edit .env:
#   SETTLEMENT_RPC_URL=http://host.docker.internal:5051
#   SETTLEMENT_ACCOUNT_ADDRESS=<from Terminal 1>
#   SETTLEMENT_ACCOUNT_PRIVATE_KEY=<from Terminal 1>
#   SETTLEMENT_CHAIN_ID=SN_SEPOLIA

# --- Build the katana-tee-mock image. First run takes a few minutes. ---
docker compose -f docker/tee-mock.compose.yml --env-file .env build

# --- Bring the stack up. Watch the logs for:
#     deploy-contracts: "TEE registry mock address: 0x..." then "Core contract address: 0x..."
#     init-chain:       "Chain spec written to /root/.config/katana/chains/katana_tee_mock"
#     katana:           "RPC server started at 0.0.0.0:5050"
#     saya-tee:         "Chain advanced" / "TEE proving completed" / "Settled block ..."
docker compose -f docker/tee-mock.compose.yml --env-file .env up

# --- Terminal 3: drive the L3, watch it settle on the L2 ---
# Chain id should come back:
curl -s http://localhost:5050 \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"starknet_chainId","params":[]}'
# The L3 rollup only advances when transactions arrive (provable mode never
# emits empty blocks). Submit any tx to make it tick.
```

Expected end state: in Terminal 1, the L2 katana shows piltover + TEE registry
deployed (two class declares + two contract deploys). In Terminal 2, saya-tee
settles blocks onto piltover; in Terminal 1 you'll see piltover's
`update_state` invocations land.

## What each service does

| Service | Runs | Purpose |
|---------|------|---------|
| `deploy-contracts` | once, exits | Declares + deploys the piltover core contract and the mock TEE registry on your L2 via `saya-ops`. Writes addresses to a shared volume. |
| `init-chain`       | once, exits | Runs `katana init rollup` with the deployed piltover address to produce the L3 chain spec. |
| `katana`           | long-running | L3 rollup node, mock TEE provider, RPC on `:5050`. |
| `saya-tee`         | long-running | Polls L3 via the compose network, builds settlement txs, submits `update_state` to piltover on your L2. |

## Resetting

- **Keep settled state, restart services:** `docker compose -f docker/tee-mock.compose.yml restart`
- **Throw away L3 chain + saya-tee cursor, keep deployed contracts:**
  `docker compose -f docker/tee-mock.compose.yml down`
- **Fresh slate (re-deploy everything):**
  `docker compose -f docker/tee-mock.compose.yml down -v`

## Image pins

- Katana builds from source (this repo) via `docker/Dockerfile.tee-mock`,
  tagged `ghcr.io/dojoengine/katana-tee-mock:dev`. Once that image is
  published to ghcr, the `build:` directive becomes cosmetic and `docker
  compose pull` works.
- `ghcr.io/dojoengine/saya:latest` is pulled for `saya-ops` and `saya-tee`.
  If behavior drifts from what this compose expects, pin to a specific
  saya release tag.

## Known gaps

- `ghcr.io/dojoengine/katana-tee-mock:<sha>` is not yet published by CI;
  first `docker compose up --build` builds the image locally. Tracked for
  a follow-up CI PR.
- Log-scraping in `deploy-contracts.sh` relies on saya-ops's `info!` log
  format. If saya-ops changes its log output, update the `extract_addr` /
  `extract_block` helpers.
- Real AMD SEV-SNP variant (`docker/tee.compose.yml`) is v2, not here.
