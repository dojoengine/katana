# Starknet Contract Deployment (sncast)

## Purpose

Deploy **any** Starknet contract instance using `sncast`. This skill is contract-agnostic and works with any class hash or contract name.

## Inputs

- `class_hash` (string) - class hash from `sncast declare`
- `contract_name` (optional string) - if using auto-declare
- `constructor_calldata` (string) - space-separated felts
- `salt` (optional) - for deterministic address
- `unique` (bool) - mix deployer address into salt
- `network` (string) - `devnet`, `sepolia`, or `mainnet`

## Deploy by Class Hash

```bash
sncast --network <network> deploy \
  --class-hash <class_hash> \
  --constructor-calldata <arg1> <arg2> ...
```

## Deploy by Contract Name (auto-declare)

```bash
sncast --network <network> deploy \
  --contract-name <ContractName> \
  --constructor-calldata <arg1> <arg2> ...
```

## Deploy with Salt / Unique

```bash
sncast --network <network> deploy \
  --class-hash <class_hash> \
  --salt <salt> --unique \
  --constructor-calldata <arg1> <arg2> ...
```

## Calldata Serialization Quick Reference

All constructor args are **felts**.

- `felt252` -> 1 felt
- `u256` -> 2 felts (low, high)
- `Array<T>` -> length + elements
- `ContractAddress` -> 1 felt

Example:

Constructor:

```
fn constructor(first: felt252, second: u256)
```

Calldata:

```
--constructor-calldata 0x1 0x2 0x0
# 0x1 = first
# 0x2 = second.low
# 0x0 = second.high
```

## Output Interpretation

`sncast` prints:
- `Contract Address`
- `Transaction Hash`

## Common Errors

- **Invalid calldata**: verify serialization for arrays/u256
- **Class hash missing**: declare first or use `--contract-name`
- **Insufficient funds**: fund deployer account on target network
