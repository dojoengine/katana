# Katana TEE

This repository contains:
- Cairo contracts for verifying AMD SEV-SNP attestation proofs on Starknet (via Garaga SP1 Groth16 verifier)
- Rust clients to fetch Katana TEE quotes, prove them with SP1, generate Starknet calldata, and invoke the on-chain verifier

## Local devnet (fork Sepolia) quickstart

### Prereqs

- `asdf` (recommended): install tool versions from `.tool-versions`

```bash
asdf install
```

### Configure environment

```bash
cp .env.example .env
```

Edit `.env` and set any RPCs/keys you need. **Do not commit `.env`** (it is gitignored).

### Start devnet (fork Sepolia)

Use any Sepolia JSON-RPC provider. For example, with the default `.env.example` variables:

```bash
set -a && . ./.env && set +a
starknet-devnet --fork-network "$STARKNET_RPC_URL_SEPOLIA" --seed "$DEVNET_SEED" --port "$DEVNET_PORT"
```

### Deploy contracts

```bash
sncast --account "$STARKNET_ACCOUNT" script run deployment --network devnet --package deployment --no-state-file
```

### Run the end-to-end pipeline (Rust CLI)

This will: fetch quote → query cache → prove → calldata → invoke `katana_tee.verify_and_update_state`.

```bash
cargo run -p katana_tee_client --bin katana-tee -- pipeline \
  --rpc http://localhost:5050 \
  --starknet-rpc http://localhost:5050 \
  --katana-tee 0x<katana_tee_contract_address> \
  --account-address 0x<starknet_account_address> \
  --account-private-key 0x<starknet_account_private_key>
```

To only generate proof + calldata (no tx):

```bash
cargo run -p katana_tee_client --bin katana-tee -- pipeline \
  --rpc http://localhost:5050 \
  --katana-tee 0x<katana_tee_contract_address> \
  --dry-run \
  --calldata-output proof_calldata.txt
```