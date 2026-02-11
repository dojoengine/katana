# Shutdown

The shard node supports graceful shutdown that ensures all in-flight work is completed and resources are cleaned up.

## Shutdown Sequence

There are two ways to initiate shutdown:

### Explicit Stop (`stop()`)

When `stop()` is called on the launched node handle:

1. **Signal the scheduler** -- the scheduler's shutdown flag is set and all blocked workers are woken.
2. **Stop the RPC server** -- the RPC server stops accepting new requests.
3. **Join worker threads** -- all worker OS threads are joined. Workers finish processing their current shard (if any), then exit when they observe the shutdown signal.
4. **Shut down the task manager** -- all async tasks (including the block context listener) are cancelled and awaited.

### Passive Shutdown (`stopped()`)

The `stopped()` method returns a future that resolves when the node's task manager signals shutdown (e.g., due to a Ctrl+C handler). When triggered:

1. **Wait for the task manager's shutdown signal**.
2. **Signal the scheduler** -- same as above.
3. **Join worker threads** -- same as above.
4. **Stop the RPC server**.

## Worker Thread Joining

Worker threads run on OS threads, not the async runtime. To avoid blocking the async runtime during shutdown, the join operation is dispatched to the task manager's blocking executor. This ensures the async shutdown sequence remains responsive while waiting for worker threads to finish.

## Ordering Guarantees

- The scheduler is signalled before workers are joined. This ensures workers observe the shutdown flag and exit their loops.
- Workers are joined before the task manager is shut down. This ensures all execution work is complete before async tasks are cancelled.
- The RPC server is stopped so no new transactions can be submitted after shutdown begins.

## Worker Behavior on Shutdown

When a worker calls `next_task()` and shutdown has been signalled, it receives `None` and exits its loop. If a worker is in the middle of executing a shard when shutdown is signalled, it finishes the current execution cycle before checking for the next task (at which point it will see the shutdown signal).
