# Starknet Account Management (sncast)

## Purpose

Create, import, list, and deploy Starknet accounts using **sncast**. This skill is **generic** and works with any project.

## Inputs

- `account_name` (string) - name stored in accounts file
- `account_type` (string) - `oz`, `argent`, or `braavos`
- `network` (string) - `devnet`, `sepolia`, or `mainnet`
- `use_predeployed_devnet` (bool) - if `true`, use `devnet-1`..`devnet-10`

## Environment Variables

These are read by `snfoundry.toml`:

- `STARKNET_NETWORK`
- `STARKNET_ACCOUNT`
- `STARKNET_ACCOUNTS_FILE`
- `STARKNET_RPC_URL_DEVNET`
- `STARKNET_RPC_URL_SEPOLIA`
- `STARKNET_RPC_URL_MAINNET`

## Predeployed Devnet Accounts

If `starknet-devnet` is running, you can use predeployed accounts directly:

```bash
sncast --account devnet-1 --network devnet account list
```

## Create Account (OpenZeppelin / Argent / Braavos)

```bash
sncast --network <network> account create \
  --name <account_name> \
  --type <account_type>
```

Notes:
- This creates local account info but does **not** deploy the account contract.
- For sepolia/mainnet, you must fund the address before deploying.

## Deploy Account Contract

```bash
sncast --network <network> account deploy \
  --name <account_name>
```

## Import Existing Account

```bash
sncast --network <network> account import \
  --name <account_name> \
  --address <account_address> \
  --private-key <private_key> \
  --type <account_type>
```

## List Accounts

```bash
sncast account list
```

## Output Interpretation

- `Address:` is the account address to fund/deploy
- `Transaction Hash:` appears after deploy

## Common Errors

- **Account not funded**: fund the address before `account deploy`
- **Invalid account type**: use `oz`, `argent`, or `braavos`
- **Missing accounts file**: ensure `STARKNET_ACCOUNTS_FILE` is set
