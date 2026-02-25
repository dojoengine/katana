# Shutdown

The shard node supports graceful shutdown that ensures all in-flight work is completed and resources are cleaned up. The `ShardRuntime` centralizes worker lifecycle management and provides two shutdown strategies.

## ShardRuntime Shutdown API

The `ShardRuntime` owns the scheduler and worker threads and exposes three ways to shut down:

- **`shutdown_timeout(duration)`** — signals the scheduler's shutdown flag, then joins each worker thread in sequence. If the deadline is exceeded, remaining worker handles are dropped and those threads detach.
- **`shutdown_background()`** — signals the scheduler and immediately drops all worker handles. Workers will observe the shutdown flag and exit on their own; no joining occurs.
- **`Drop`** — if the runtime has not been consumed by one of the above methods, the `Drop` impl signals the scheduler and blocks until all workers exit (similar to `tokio::runtime::Runtime`'s drop behavior).

## Shutdown Sequence

There are two ways to initiate shutdown:

### Explicit Stop (`stop()`)

When `stop()` is called on the launched node handle:

1. **Stop the RPC server** -- the RPC server stops accepting new requests.
2. **Shut down the runtime** -- the runtime is taken from the node and `shutdown_timeout(30s)` is called on a blocking thread (via the task manager's blocking executor) to avoid blocking the async runtime.
3. **Shut down the task manager** -- all async tasks (including the block context listener) are cancelled and awaited.

### Passive Shutdown (`stopped()`)

The `stopped()` method returns a future that resolves when the node's task manager signals shutdown (e.g., due to a Ctrl+C handler). When triggered:

1. **Wait for the task manager's shutdown signal**.
2. **Shut down the runtime** -- the runtime is taken and `shutdown_timeout(30s)` is called on a blocking thread.
3. **Stop the RPC server**.

## Worker Thread Joining

Worker threads run on OS threads, not the async runtime. To avoid blocking the async runtime during shutdown, the join operation is dispatched to the task manager's blocking executor. This ensures the async shutdown sequence remains responsive while waiting for worker threads to finish.

## Ordering Guarantees

- The scheduler is signalled before workers are joined. This ensures workers observe the shutdown flag and exit their loops.
- Workers are joined before the task manager is shut down. This ensures all execution work is complete before async tasks are cancelled.
- The RPC server is stopped so no new transactions can be submitted after shutdown begins.

## Worker Behavior on Shutdown

When a worker calls `next_task()` and shutdown has been signalled, it receives `None` and exits its loop. If a worker is in the middle of executing a shard when shutdown is signalled, it finishes the current execution cycle before checking for the next task (at which point it will see the shutdown signal).
