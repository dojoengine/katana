# Starknet Contract Declaration (sncast)

## Purpose

Declare **any** Starknet contract class using `sncast`. This skill is contract-agnostic and works with any Scarb project or workspace.

## Inputs

- `contract_name` (string) - name after `mod` in Cairo contract
- `package_name` (optional string) - Scarb package name in workspace
- `network` (string) - `devnet`, `sepolia`, or `mainnet`

## Preconditions

- `sncast` installed
- Scarb project present (sncast builds automatically)
- `snfoundry.toml` configured (env vars supported)

## Declare Contract (simple)

```bash
sncast --network <network> declare \
  --contract-name <ContractName>
```

## Declare Contract (workspace package)

```bash
sncast --network <network> declare \
  --contract-name <ContractName> \
  --package <package_name>
```

## Output Interpretation

`sncast` prints:
- `Class Hash` (or `class_hash`) - use this for deploy
- `Transaction Hash`

## Idempotency

If already declared:
- sncast returns the existing class hash
- treat as success

## Common Errors

- **Wrong contract name**: must match the `mod ContractName` in Cairo code
- **Missing Scarb.toml**: run in project root or a parent
- **Compilation failure**: fix Cairo errors reported by scarb
