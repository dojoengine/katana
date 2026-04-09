# OpenZeppelin Universal Deployer Contract (mainnet binary)

This directory vendors the Sierra contract class for the new OpenZeppelin Universal Deployer
that is currently deployed on Starknet mainnet.

- **Sierra class hash:** `0x01b2df6d8861670d4a8ca4670433b2418d78169c2947f46dc614e69f333745c8`
- **Sierra version:** 1.7.0
- **Source contract:** `presets/src/universal_deployer.cairo` from
  [OpenZeppelin/cairo-contracts](https://github.com/OpenZeppelin/cairo-contracts), conceptually
  the v2.0.0 / v3.0.0-alpha.0 source (the source file is byte-identical between those tags).

## Why a vendored binary instead of a source build?

The Sierra hash above is the one OpenZeppelin documents at the v3.0.0-alpha.0 tag and is the
class actually registered on mainnet. We could not reproduce it locally with any scarb release
we tried (`2.11.4`, `2.12.2`, `2.13.1`, `2.15.0`) — every scarb version produces a different
hash from byte-identical source, because the bundled cairo-lang snapshot drifts release over
release. To guarantee that Katana's predeployed UDC class-hash-equals the mainnet contract, we
fetch the canonical artifact directly instead of relying on a recompile.

## How the file was produced

```bash
curl -s -X POST https://rpc.starknet.lava.build \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"starknet_getClass","params":["latest","0x01b2df6d8861670d4a8ca4670433b2418d78169c2947f46dc614e69f333745c8"]}' \
  | jq '.result | .abi |= fromjson' \
  > UniversalDeployer.contract_class.json
```

The only post-processing applied is parsing the `abi` field from a JSON-encoded string into a
JSON array, which matches the on-disk shape Scarb emits and is what
`katana_primitives::class::ContractClass::from_str` expects. This transformation does not
affect the computed class hash (the hash is over the raw ABI string), as verified by the
`openzeppelin_udc_hash_matches_mainnet` test in `crates/contracts/src/lib.rs`.

## Updating

If OpenZeppelin ships a new UDC and mainnet adopts it, refetch with the new class hash, drop
the JSON in this directory, and update the pinned hash in the test referenced above. There is
no scarb step.
