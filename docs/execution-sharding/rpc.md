# Shard RPC API

The shard node exposes a JSON-RPC interface under the `shard` namespace. This API mirrors a subset of the standard Starknet RPC methods, with an additional `shard_id` parameter that routes each request to the appropriate shard.

## Namespace

All methods are prefixed with `shard_`. For example: `shard_getBlockWithTxHashes`, `shard_addInvokeTransaction`.

## Request Routing

Every method (except `shard_chainId` and `shard_listShards`) takes a `shard_id` parameter as its first argument. This is a contract address that identifies the target shard.

- **Read operations** look up the shard in the registry. If the shard does not exist, an `INVALID_PARAMS` error is returned.
- **Write operations** look up or create the shard in the registry. If the shard does not exist, it is created lazily before the transaction is added. After the transaction is added to the shard's pool, the shard is scheduled for execution.

## Methods

### Management

| Method | Parameters | Description |
|---|---|---|
| `shard_listShards` | (none) | Returns a list of all currently active shard IDs |
| `shard_chainId` | (none) | Returns the chain ID (not shard-specific) |

### Read

These methods query a specific shard's storage. They require the shard to already exist.

| Method | Key Parameters | Description |
|---|---|---|
| `shard_getBlockWithTxHashes` | `shard_id`, `block_id` | Block info with tx hashes |
| `shard_getBlockWithTxs` | `shard_id`, `block_id` | Block info with full transactions |
| `shard_getStorageAt` | `shard_id`, `contract_address`, `key`, `block_id` | Storage value |
| `shard_getNonce` | `shard_id`, `block_id`, `contract_address` | Account nonce |
| `shard_getTransactionByHash` | `shard_id`, `transaction_hash` | Transaction details |
| `shard_getTransactionReceipt` | `shard_id`, `transaction_hash` | Transaction receipt |
| `shard_getTransactionStatus` | `shard_id`, `transaction_hash` | Transaction status |
| `shard_call` | `shard_id`, `request`, `block_id` | Call without creating a tx |
| `shard_getEvents` | `shard_id`, `filter` | Event log query |
| `shard_estimateFee` | `shard_id`, `request`, `simulation_flags`, `block_id` | Fee estimation |
| `shard_blockHashAndNumber` | `shard_id` | Latest block hash and number |
| `shard_blockNumber` | `shard_id` | Latest block number |
| `shard_getStateUpdate` | `shard_id`, `block_id` | State update for a block |

### Write

These methods submit transactions to a shard. They create the shard if it does not yet exist and schedule it for execution after the transaction is accepted.

| Method | Key Parameters | Description |
|---|---|---|
| `shard_addInvokeTransaction` | `shard_id`, `invoke_transaction` | Submit an invoke transaction |
| `shard_addDeclareTransaction` | `shard_id`, `declare_transaction` | Submit a declare transaction |
| `shard_addDeployAccountTransaction` | `shard_id`, `deploy_account_transaction` | Submit a deploy account transaction |

### Trace

| Method | Key Parameters | Description |
|---|---|---|
| `shard_traceTransaction` | `shard_id`, `transaction_hash` | Execution trace of a transaction |

## Error Handling

| Condition | Error Code | Message |
|---|---|---|
| Read to a non-existent shard | `INVALID_PARAMS` | "Shard not found" |
| Failed to create a shard | `INTERNAL_ERROR` | "Failed to create shard: ..." |
| Shard-internal errors | (varies) | Propagated from the underlying Starknet API |

## Relationship to Standard Starknet API

The shard API methods have the same semantics as their Starknet counterparts. The only difference is the additional `shard_id` routing parameter and the `shard_` namespace prefix. Response types are identical.
