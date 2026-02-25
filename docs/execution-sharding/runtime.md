# ShardRuntime

The `ShardRuntime` is the single owner of the shard node's execution resources: the scheduler and the pool of worker OS threads. It provides a tokio-style lifecycle API for starting, running, and shutting down the worker pool.

## Design

The runtime is split into two types:

- **`ShardRuntime`** — owns the worker thread handles and provides lifecycle methods (`start`, `shutdown_timeout`, `shutdown_background`). Not cloneable. Analogous to `tokio::runtime::Runtime`.
- **`RuntimeHandle`** — a cheap, cloneable reference to the scheduler and shared block environment. Used by the RPC layer and other components to schedule work without owning the workers. Analogous to `tokio::runtime::Handle`.

## Lifecycle

1. **`ShardRuntime::new(worker_count, time_quantum, block_env)`** — creates the runtime and its internal scheduler. No threads are spawned yet.
2. **`runtime.handle()`** — returns a `RuntimeHandle` that can be cloned and passed to components that need to schedule shards (e.g., the RPC provider).
3. **`runtime.start()`** — spawns the worker OS threads. Must be called exactly once.
4. **Shutdown** — see [Shutdown](shutdown.md) for details on the three shutdown strategies.

## RuntimeHandle

The `RuntimeHandle` exposes:

- **`schedule(shard)`** — delegates to the scheduler's `schedule()` method.
- **`block_env()`** — returns a reference to the shared `Arc<RwLock<BlockEnv>>`.
- **`scheduler()`** — returns a reference to the underlying `ShardScheduler` for direct access when needed.

## Ownership

During `Node::build()`, the runtime is created and stored in the `Node`. On `Node::launch()`, the runtime is moved into `LaunchedShardNode`, which uses it for shutdown. The `RuntimeHandle` remains in `Node` and is accessible for the node's lifetime.
