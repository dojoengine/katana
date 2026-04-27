# Saya

Saya is the proving and settlement orchestrator for the Dojo/Katana stack. It reads finalized blocks from a Katana rollup node and settles them on-chain.

---

## Installation

```bash
asdf plugin add saya https://github.com/dojoengine/asdf-saya.git
asdf install saya latest
```

This installs three commands: `saya`, `saya-ops`, and `saya-tee`.

Alternatively, grab pre-built binaries from the [releases page](https://github.com/dojoengine/saya/releases) or use the Docker image `ghcr.io/dojoengine/saya`.

---

## Overview

### How it works

```
Katana L3 node  ──blocks──▶  Saya  ──proof + state──▶  Piltover (L2)
```

Saya polls Katana for finalized blocks, proves them (via Atlantic or TEE), and submits state updates to the Piltover settlement contract on L2.

### Modes

| Mode | Binary | How it proves | Settlement |
|------|--------|---------------|------------|
| Persistent | `persistent` | STARK proof via Atlantic (Herodotus) | Piltover on L2 |
| Sovereign | `persistent` | STARK proof via Atlantic | Celestia DA (no on-chain settlement) |
| Persistent-TEE | `persistent-tee` | AMD SEV-SNP attestation → SP1 Groth16 | Piltover on L2 |

### What you need per mode

| Requirement | Persistent | Sovereign | Persistent-TEE |
|-------------|:---:|:---:|:---:|
| Katana in provable mode | ✓ | ✓ | ✓ |
| Piltover contract deployed | ✓ | — | ✓ |
| Settlement account (L2) | ✓ | — | ✓ |
| Atlantic API key | ✓ | ✓ | — |
| `layout_bridge` program | ✓ | ✓ | — |
| Katana-TEE node | — | — | ✓ |
| TEE registry contract | — | — | ✓ |
| Prover network account | — | — | ✓ |
| Celestia light node | — | ✓ | — |

---

## Setup

These are one-time steps before running Saya.

### 1. Deploy Piltover (`ops`)

Set environment variables:

```bash
export SETTLEMENT_ACCOUNT_PRIVATE_KEY=<PRIVATE_KEY_IN_HEX>
export SETTLEMENT_ACCOUNT_ADDRESS=<ACCOUNT_ADDRESS_IN_HEX>
export SETTLEMENT_CHAIN_ID=<sepolia|mainnet|CUSTOM_CHAIN_ID>
# SETTLEMENT_RPC_URL is optional for sepolia/mainnet (default public RPC is used)
```

Then declare, deploy, and configure the contract:

```bash
saya-ops core-contract declare
saya-ops core-contract deploy --salt 0x5
saya-ops core-contract setup-program \
  --core-contract-address <ADDRESS_FROM_DEPLOY> \
  --chain-id example-chain
```

The `deploy` command prints the **contract address** and **deployed block number** — save both for the next step.

> For local testing without real proofs, deploy a mock fact registry instead:
> ```bash
> saya-ops core-contract declare-and-deploy-fact-registry-mock --salt 0x1
> # then pass --fact-registry-address <MOCK_ADDRESS> to setup-program
> ```

### 2. Initialize Katana

Katana must run in **provable mode** (supported from Dojo `1.3.0`, Starknet v14.0.1 only).

```bash
katana init \
  --settlement-chain sepolia \
  --id example-chain \
  --settlement-contract <PILTOVER_ADDRESS> \
  --settlement-contract-deployed-block <DEPLOYED_BLOCK>
```

Start Katana with a block time (required for provable mode — Katana never produces empty blocks):

```bash
katana --chain example-chain --block-time 30000
```

### 3. Get Cairo programs (persistent mode only)

The `layout_bridge` program ships with the release. Download it from the [releases page](https://github.com/dojoengine/saya/releases) or build it locally:

```bash
./scripts/generate_layout_bridge.sh
# Requires Docker (linux/amd64 only — emulation needs ~32 GB RAM)
```

---

## Running

### Persistent mode

```bash
saya start \
  --rollup-rpc http://localhost:5050 \
  --settlement-rpc https://starknet-sepolia.public.blastapi.io \
  --settlement-piltover-address <PILTOVER_ADDRESS> \
  --settlement-account-address <ACCOUNT_ADDRESS> \
  --settlement-account-private-key <PRIVATE_KEY> \
  --layout-bridge-program programs/layout_bridge.json \
  --atlantic-key <ATLANTIC_KEY> \
  --settlement-integrity-address <INTEGRITY_ADDRESS>
```

<details>
<summary>All options</summary>

```
--rollup-rpc <URL>                           Katana L3 JSON-RPC endpoint
--settlement-rpc <URL>                       Settlement chain JSON-RPC endpoint
--settlement-piltover-address <FELT>         Piltover contract address
--settlement-account-address <FELT>          Submitter account address
--settlement-account-private-key <FELT>      Submitter account private key
--layout-bridge-program <PATH>               Path to compiled layout_bridge program
--atlantic-key <KEY>                         Atlantic (Herodotus) API key
--settlement-integrity-address <FELT>        On-chain integrity/fact registry address
--blocks-processed-in-parallel <N>           Parallel block pipeline depth (default: 60)
--db-dir <PATH>                              SQLite database directory
--mock-layout-bridge-program-hash <HASH>     Skip real Atlantic proving (testing only)
--mock-snos-from-pie                         Derive SNOS proof from PIE (testing only)
```

</details>

### Sovereign mode

```bash
saya sovereign start \
  --rollup-rpc http://localhost:5050 \
  --celestia-rpc http://localhost:26658 \
  --celestia-token <TOKEN>
```

A helper script for running a local Celestia light node is at `scripts/celestia.sh`.

### Persistent-TEE mode

```bash
saya-tee tee start \
  --rollup-rpc http://localhost:5050 \
  --settlement-rpc https://starknet-sepolia.public.blastapi.io \
  --settlement-piltover-address <PILTOVER_ADDRESS> \
  --settlement-account-address <ACCOUNT_ADDRESS> \
  --settlement-account-private-key <PRIVATE_KEY> \
  --tee-registry-address <TEE_REGISTRY_ADDRESS> \
  --prover-private-key <PROVER_PRIVATE_KEY>
```

<details>
<summary>All options</summary>

```
--rollup-rpc <URL>                       Katana TEE node JSON-RPC endpoint
--settlement-rpc <URL>                   Settlement chain JSON-RPC endpoint
--settlement-piltover-address <FELT>     Piltover contract address
--settlement-account-address <FELT>      Submitter account address
--settlement-account-private-key <FELT>  Submitter account private key
--tee-registry-address <FELT>            TEE registry contract on the prover network
--prover-private-key <STRING>            Prover network account private key
--batch-size <N>                         Blocks per attestation batch (default: 10)
--idle-timeout-secs <N>                  Flush partial batch after N idle seconds (default: 120)
--attestor-poll-interval-ms <N>          Attestor poll interval in ms (default: 1000)
--db-dir <PATH>                          SQLite database directory
```

</details>

---

## Building from source

Only needed if you want to modify Saya.

**One-time setup:**

```bash
git clone --recursive https://github.com/dojoengine/saya
# or, if you already cloned without --recursive:
# git submodule update --init --recursive

make install-scarb
```

`make install-scarb` uses [asdf](https://asdf-vm.com/) to install the `scarb`
version pinned by `piltover/.tool-versions`. `saya-ops` rebuilds
two Piltover mock contracts from that submodule on every cargo build via its
`build.rs`, so scarb is a hard build-time dependency.

Rust `1.89` is also required — install via [rustup](https://rustup.rs);
`rust-toolchain.toml` pins the version automatically.

**Build:**

```bash
cd bin/persistent && cargo build --release
cd bin/ops && cargo build --release
cd bin/persistent-tee && cargo build --release
```

---

## Testing

```bash
# Unit + integration tests
cargo test --workspace --all-features

# E2E (requires Docker)
docker compose -f compose.e2e.yml up
```
