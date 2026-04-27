# Katana TEE RPC Client Setup

This page documents how to use the existing `katana-tee-setup.sh` helper for connecting to a remote Katana node running inside an AMD SEV-SNP protected VM. The canonical entrypoint is `README.md`.

## Prerequisites

- SSH access to the TEE host server
- `sshpass` installed locally (for automated SSH): `apt install sshpass` or `brew install hudochenkov/sshpass/sshpass`

## Configure `.env`

Copy `.env.example` and fill in the TEE-related variables:

```bash
cp .env.example .env
```

Required keys:
- `TEE_HOST`
- `TEE_SSH_USER` and either `TEE_SSH_KEY` or `TEE_SSH_PASSWORD`
- `VM_SSH_PORT`
- `RPC_PORT`

## Usage

```bash
./katana-tee-setup.sh start   # start VM + Katana, prints RPC URL
./katana-tee-setup.sh status  # check status
./katana-tee-setup.sh test    # test TEE attestation endpoint
./katana-tee-setup.sh url     # print RPC URL only
./katana-tee-setup.sh stop    # stop VM
```

## Output

`./katana-tee-setup.sh start` prints the RPC URL and writes it to `.katana-rpc-url` for use by other scripts.

## Using the RPC

```bash
curl -s "http://$TEE_HOST:$RPC_PORT" -X POST \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"tee_generateQuote","params":[],"id":1}'
```
