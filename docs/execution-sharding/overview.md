# Execution Sharding

## Overview

The execution sharding node is an alternative node type that partitions transaction execution across isolated **shards**, where each shard corresponds to a single contract address. Transactions targeting different contracts execute in parallel on separate shards, each with its own storage, transaction pool, and query interface.

This design enables horizontal scaling of transaction throughput: shards that touch different contracts never contend with each other, and the number of worker threads can scale with available CPU cores.

## Key Concepts

### Shard

A shard is the fundamental unit of isolation. Each shard is identified by a `ContractAddress` and contains:

- **Isolated storage** -- an independent database holding the shard's full blockchain state, initialized from the chain's genesis block.
- **Transaction pool** -- a per-shard mempool that buffers incoming transactions until a worker executes them.
- **Query interface** -- a per-shard API handler that serves read requests (blocks, receipts, state) from the shard's own storage.

Shards are completely independent of each other. A transaction submitted to shard A is invisible to shard B.

### Shard Lifecycle States

Each shard is in exactly one of three states at any point in time:

```
Idle --> Pending --> Running --> Idle
```

- **Idle** -- the shard has no pending work and is not enqueued. This is the only state from which a shard can be scheduled.
- **Pending** -- the shard has been enqueued in the scheduler's work queue but has not yet been picked up by a worker.
- **Running** -- a worker is actively executing transactions for this shard.

State transitions are atomic to prevent double-scheduling: only one `Idle -> Pending` transition can succeed for a given shard at a time.

### No Block Producer

Unlike the standard sequencer node, shards do **not** have a block producer. Workers execute transactions directly and commit results to storage immediately. Each execution batch is persisted as a "block" in the shard's storage, making transactions, receipts, and traces queryable as soon as execution completes.

As a consequence, there is no concept of a "pending block" within a shard.

### Shard Creation

How shards are created is determined by the **shard manager** (`ShardManager` trait), which acts as both the shard store and the creation policy:

- **Lazy (dev mode)** — `LazyShardManager` creates shards on demand when a transaction is first submitted to a given contract address. This avoids pre-allocating resources for contracts that may never receive traffic.
- **Onchain (production)** — a future `OnchainShardManager` would only look up pre-registered shards; creation is driven by onchain events rather than RPC requests.

Once created, a shard persists for the lifetime of the node regardless of the manager implementation.

## Architecture

```
                          +------------------+
                          |    RPC Server    |
                          |  (shard namespace)|
                          +--------+---------+
                                   |
                          +--------v---------+
                          |  Shard Manager    |
                          | (pluggable policy)|
                          +--------+---------+
                                   |
              +--------------------+--------------------+
              |                    |                    |
         +----v----+         +----v----+         +----v----+
         | Shard A |         | Shard B |         | Shard C |
         | storage |         | storage |         | storage |
         | pool    |         | pool    |         | pool    |
         +---------+         +---------+         +---------+
              |                    |                    |
              +--------------------+--------------------+
                                   |
                          +--------v---------+
                          |    Scheduler     |
                          |  (FIFO queue)    |
                          +--------+---------+
                                   |
              +--------------------+--------------------+
              |                    |                    |
         +----v----+         +----v----+         +----v----+
         | Worker  |         | Worker  |         | Worker  |
         | Thread 0|         | Thread 1|         | Thread N|
         +---------+         +---------+         +---------+
```

### Transaction Flow

1. A client submits a transaction via `shard_addInvokeTransaction(shard_id, tx)`.
2. The RPC layer looks up (or creates) the shard for `shard_id` and adds `tx` to the shard's pool.
3. The RPC layer schedules the shard -- if it is `Idle`, it transitions to `Pending` and is enqueued.
4. A worker thread dequeues the shard, transitions it to `Running`, and begins executing pending transactions.
5. After execution, results are committed to the shard's storage as a new block.
6. The worker checks whether the shard still has pending transactions:
   - If yes: the shard is re-scheduled (set back to `Idle` then enqueued as `Pending`).
   - If no: the shard transitions to `Idle`.

### Shared vs Per-Shard Resources

| Resource | Scope | Rationale |
|---|---|---|
| Executor factory | Shared | Global class cache, execution config |
| Chain spec | Shared | Same chain identity across all shards |
| Gas price oracle | Shared | Uniform gas pricing |
| Block environment | Shared | All shards use the same block context |
| Database | Per-shard | Storage isolation |
| Transaction pool | Per-shard | Per-contract tx ordering |
| Backend | Per-shard | Owns per-shard storage |

## Running the Shard Node

The shard node is launched via the `katana node shard` subcommand. The only required argument is the base chain URL, which points to the Starknet RPC endpoint that the shard node polls for block context updates.

```bash
katana node shard --base-chain-url http://localhost:5050
```

### Key Options

| Option | Default | Description |
|---|---|---|
| `--base-chain-url <URL>` | *(required)* | RPC URL of the base/settlement chain |
| `--workers <COUNT>` | CPU core count | Number of shard worker threads |
| `--time-quantum <MS>` | 100 | Worker preemption time quantum in milliseconds |
| `--block-poll-interval <SECS>` | 12 | How often to poll the base chain for new blocks |
| `--no-fee` | false | Disable transaction fee charging |
| `--no-account-validation` | false | Disable account validation |
| `--http.addr <ADDR>` | 0.0.0.0 | RPC server listen address |
| `--http.port <PORT>` | 5050 | RPC server listen port |

### Example

Start a shard node with 4 workers and fees disabled, connected to a local sequencer:

```bash
katana node shard \
  --base-chain-url http://localhost:5050 \
  --workers 4 \
  --no-fee \
  --http.port 6060
```

## Components

Each component is documented in its own file:

- [Shard](shard.md) -- per-contract isolated execution unit, block environment
- [Shard Manager](manager.md) -- shard lookup and pluggable creation policy
- [Scheduler and Workers](scheduler.md) -- work queue, OS thread pool, execution loop
- [RPC API](rpc.md) -- the `shard` JSON-RPC namespace
- [Shutdown](shutdown.md) -- graceful shutdown behavior
