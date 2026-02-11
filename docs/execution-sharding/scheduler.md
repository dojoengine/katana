# Scheduler and Workers

The scheduler is a shared work queue that distributes shards to a pool of worker threads for execution. The RPC layer enqueues shards, and dedicated OS threads dequeue and process them.

## Scheduling

When a shard is scheduled:

1. The scheduler atomically attempts to transition the shard from `Idle` to `Pending`.
2. If the transition succeeds, the shard is appended to the back of the queue and one waiting worker is woken.
3. If the transition fails (the shard is already `Pending` or `Running`), the call is a no-op.

This atomic state check prevents a shard from being enqueued multiple times. A shard can only enter the queue from the `Idle` state. The queue is strictly FIFO: shards are processed in the order they were scheduled.

## Workers

Each worker is a dedicated OS thread that runs independently. Workers run on OS threads (not async tasks) because transaction execution is CPU-bound and should not block the async runtime. The number of worker threads defaults to the number of available CPU cores.

### Worker Loop

1. The worker blocks on the scheduler waiting for a shard (or a shutdown signal).
2. When a shard is received, the worker transitions it to `Running`.
3. The worker enters an execution loop for that shard (see below).
4. When the loop ends, the worker either re-schedules the shard or marks it `Idle`.
5. The worker returns to step 1.

### Execution Loop

For each shard it picks up, the worker repeats the following cycle:

1. **Collect** -- take a snapshot of all pending transactions from the shard's pool.
2. **Execute** -- create an executor with the shard's latest state and the shared block environment, then execute the collected transactions.
3. **Commit** -- persist the execution results (state diff, receipts, traces) to the shard's storage as a new block.
4. **Remove** -- remove the executed transactions from the shard's pool.
5. **Check quantum** -- if the worker has been processing this shard for longer than the time quantum, yield by breaking out of the loop.

If there are no pending transactions at step 1, the loop exits immediately.

### Post-Processing

After the execution loop ends (either because the pool is empty or the quantum expired):

- **If the shard still has pending transactions**: the shard is transitioned to `Idle` and immediately re-scheduled. This ensures it re-enters the queue and will be picked up for further processing.
- **If the shard's pool is empty**: the shard is transitioned to `Idle` and left dormant until a new transaction arrives.

### Block Environment

All workers share a single block environment (`BlockEnv`) that contains the current block number, timestamp, gas prices, and sequencer address. Workers read this value at the start of each execution cycle. The block environment is updated by the [Block Context Listener](block-context-listener.md).

### Error Handling

If execution or commit fails for a batch of transactions, the error is logged but the worker continues. The failed transactions are still removed from the pool. The worker then proceeds to the next cycle or the next shard.

## Time Quantum

The scheduler holds a configurable **time quantum** -- the maximum duration a worker should spend processing a single shard before yielding. This prevents a shard with a large backlog from starving other shards. Workers check elapsed time after each execution cycle and yield if the quantum is exceeded.

When preempted, the shard is re-scheduled so it will be picked up again (by the same or a different worker).

The default time quantum is 100 milliseconds.

## Concurrency

- Workers are independent of each other and never share a shard simultaneously. A shard's scheduling state (`Idle`/`Pending`/`Running`) guarantees mutual exclusion: only one worker processes a given shard at a time.
- The scheduler is cloneable; all clones share the same underlying queue. It is safe to call `schedule()` from any thread (e.g., from RPC handlers) while workers block on their own threads.

## Shutdown

When shutdown is signalled:

1. A shutdown flag is set atomically.
2. All workers currently blocking on the scheduler are woken.
3. Each woken worker observes the shutdown flag and exits its loop.
4. Subsequent calls to dequeue immediately return `None`.
5. Worker threads are joined to ensure clean termination before the node finishes shutting down.
