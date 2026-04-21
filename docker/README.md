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
- An L2 settlement layer you can write to:
  - **Starknet Sepolia** is the easiest. Fund a Starknet account with a bit
    of STRK from the [faucet](https://starknet-faucet.vercel.app/).
  - Starknet mainnet if you're feeling spicy.
  - A local Starknet devnet (stock `katana` on port 5051, or `starknet-devnet`,
    etc.). Reach it from the compose network via `http://host.docker.internal:<port>`.

## Quickstart

```bash
# 1. Copy the example env file and fill in your L2 account details.
cp docker/tee-mock.env.example .env
$EDITOR .env

# 2. Build + start. First build takes a few minutes (cargo + Rust stdlib);
#    subsequent runs reuse the image.
docker compose -f docker/tee-mock.compose.yml --env-file .env up --build

# 3. In another terminal, drive the L3 at localhost:5050. Watch saya-tee
#    in the compose logs settle each block onto your L2.
curl -s http://localhost:5050 \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"starknet_chainId","params":[]}'
```

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
