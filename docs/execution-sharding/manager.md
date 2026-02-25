# Shard Manager

The shard manager is the central directory of all active shards and the policy that governs how shards are created. It maps shard IDs (contract addresses) to shard instances via the `ShardManager` trait.

## `ShardManager` Trait

```rust
pub trait ShardManager: Send + Sync + Debug + 'static {
    fn get(&self, id: ShardId) -> Result<Arc<Shard>>;
    fn shard_ids(&self) -> Vec<ShardId>;
}
```

- **`get(id)`** -- resolve a shard by ID. What happens on a cache miss is implementation-defined: a lazy manager creates the shard, a strict manager returns an error.
- **`shard_ids()`** -- list all currently registered shard IDs.

The RPC layer accesses shards exclusively through `dyn ShardManager`, making the creation policy transparent to callers.

## Implementations

### `LazyShardManager` (dev mode)

The default implementation used during local development. When `get()` is called for an unknown
shard ID, it creates the shard on the fly using shared resources (chain spec, executor factory,
gas oracle, task spawner, base chain RPC endpoint) provided at construction time.

Before creating the shard, the manager fetches the latest block from the base chain and derives
the shard's initial `BlockEnv` from it. If the fetch fails, `get()` returns an error and no shard
is registered.

This "get or create" behavior is idempotent: concurrent requests for the same shard ID all receive the same shard instance thanks to double-checked locking.

#### Creation Guarantees

- A shard ID maps to at most one shard instance for the lifetime of the node.
- Two concurrent `get` calls for the same ID both return the same shard.
- Shared resources are provided at manager creation time and are immutable thereafter.

### Future: `OnchainShardManager` (production)

In production, shards are not created by RPC requests. Instead, an onchain event listener calls a concrete `register(id)` method on the manager when a new shard is announced onchain. The trait `get()` method only performs lookups and returns an error for unknown shard IDs.

## Thread Safety

All `ShardManager` implementations are safe to use from multiple threads concurrently:

- Read operations (lookup, enumeration) do not block each other.
- Write operations (creation/registration) are serialized but do not block concurrent reads.

## Lifetime

The manager exists for the lifetime of the shard node. Shards are never removed from the manager once created.
