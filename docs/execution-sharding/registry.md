# Shard Registry

The shard registry is the central directory of all active shards. It maps shard IDs (contract addresses) to shard instances and is responsible for lazy shard creation.

## Responsibilities

1. **Lookup** -- retrieve an existing shard by its ID. Returns `None` if the shard does not exist.
2. **Lazy creation** -- retrieve an existing shard or create a new one if it doesn't exist yet. Used by write operations to ensure a shard exists before accepting transactions.
3. **Enumeration** -- list all currently registered shard IDs.

## Lazy Creation

Shards are not pre-allocated. The first time a write operation targets a contract address that has no shard, the registry creates one on the fly. This "get or create" operation is idempotent: concurrent requests for the same shard ID will all receive the same shard instance.

The creation process is serialized to prevent duplicate shards. Concurrent readers are not blocked while a new shard is being created (only writers contend).

### Creation Guarantees

- A shard ID maps to at most one shard instance for the lifetime of the node.
- Two concurrent `get_or_create` calls for the same ID both return the same shard.
- The registry holds shared resources (chain spec, executor factory, gas oracle, task spawner) needed to construct new shards. These resources are provided at registry creation time and are immutable thereafter.

## Thread Safety

The registry is safe to use from multiple threads concurrently:

- Read operations (lookup, enumeration) do not block each other.
- Write operations (creation) are serialized but do not block concurrent reads.
- The registry is cloneable; all clones share the same underlying state.

## Lifetime

The registry exists for the lifetime of the shard node. Shards are never removed from the registry once created.
