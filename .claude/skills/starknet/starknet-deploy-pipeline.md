# Starknet Deployment Pipeline (sncast, no custom CLI)

## Purpose

Orchestrate multi-contract deployments with dependencies using **only sncast commands**. This skill generates an ordered sequence of `sncast declare` + `sncast deploy` commands and explains how to wire dependency addresses into constructor calldata.

## Inputs

- `manifest` (user-provided) describing contracts and dependencies
- `network` (`devnet`, `sepolia`, `mainnet`)
- `account` (if not using devnet predeployed)

## Recommended Manifest Format (JSON)

```json
{
  "network": "devnet",
  "contracts": [
    {
      "name": "ContractA",
      "package": "my_package",
      "constructor_calldata": []
    },
    {
      "name": "ContractB",
      "package": "my_package",
      "depends_on": ["ContractA"],
      "constructor_calldata": ["${ContractA.address}", "0x1"]
    }
  ]
}
```

## Pipeline Steps (What the Skill Generates)

1. **Sort by dependencies** (ContractA → ContractB)
2. **Declare each contract** (optional if using `--contract-name` deploy)
3. **Deploy each contract** with resolved calldata
4. **Record outputs** into `deployments/<network>.json`

## Example Generated Commands

Assume:
- `ContractA` has no constructor args
- `ContractB` expects `ContractA.address` as first arg

### Declare

```bash
sncast --network devnet declare --contract-name ContractA --package my_package
sncast --network devnet declare --contract-name ContractB --package my_package
```

### Deploy

```bash
# ContractA
sncast --network devnet deploy --contract-name ContractA

# ContractB (replace with ContractA address returned above)
sncast --network devnet deploy --contract-name ContractB \
  --constructor-calldata <ContractA.address> 0x1
```

## Recording Deployment Output

After each deploy, capture:
- `class_hash`
- `contract_address`
- `tx_hash`

Write to:

```
deployments/<network>.json
```

Example output:

```json
{
  "network": "devnet",
  "contracts": {
    "ContractA": {
      "class_hash": "0x...",
      "address": "0x...",
      "tx_hash": "0x..."
    },
    "ContractB": {
      "class_hash": "0x...",
      "address": "0x...",
      "tx_hash": "0x..."
    }
  }
}
```

## Idempotency Guidance

If `deployments/<network>.json` already has an address for a contract, skip redeploying it. Only deploy missing contracts.

## Common Errors

- **Wrong dependency order**: ensure topological sorting
- **Incorrect calldata substitution**: replace placeholders with actual addresses
- **Insufficient funds**: fund the deployer account
