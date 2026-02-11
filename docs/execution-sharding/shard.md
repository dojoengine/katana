# Shard

A shard is the fundamental unit of execution isolation. Each shard is identified by a contract address and encapsulates an independent blockchain-like environment: its own storage, transaction pool, backend, and query interface.

## Identity

A shard is identified by a `ShardId`, which is a `ContractAddress`. This address represents the contract whose transactions are routed to this shard.

## Initialization

When a shard is created, it performs the following setup:

1. **Allocate isolated storage** -- a fresh in-memory database that is independent of all other shards.
2. **Initialize genesis** -- the chain's genesis state (accounts, classes, allocations) is written to the shard's storage. Every shard starts with an identical copy of the genesis state.
3. **Create a transaction pool** -- a FIFO-ordered pool with a transaction validator built from the shard's latest state.
4. **Create a query interface** -- enables the RPC layer to serve read requests from this shard's storage.

## State Machine

Each shard maintains an atomic scheduling state with three possible values:

| State | Meaning |
|---|---|
| `Idle` | No pending work; not in the scheduler queue |
| `Pending` | Enqueued in the scheduler; waiting for a worker |
| `Running` | A worker is actively processing this shard |

### Valid Transitions

- `Idle -> Pending` -- occurs when the shard is scheduled (a transaction was submitted). This transition is atomic: if two threads race to schedule the same shard, only one succeeds.
- `Pending -> Running` -- occurs when a worker dequeues and begins processing the shard (the worker sets the state after dequeue).
- `Running -> Idle` -- occurs when the worker finishes and the shard has no remaining pending transactions.
- `Running -> Idle -> Pending` -- occurs when the worker finishes but the shard still has pending transactions. The worker resets to `Idle` and immediately re-schedules, which transitions to `Pending`.

The `Idle -> Pending` transition uses an atomic compare-and-swap. This is the mechanism that **prevents double-scheduling**: if a shard is already `Pending` or `Running`, scheduling is a no-op.

## No Pending Block

Shards do not have a block producer. The worker executes transactions and commits results directly to storage. As a result, queries for "pending" state always return `None`. All data becomes queryable as committed blocks immediately after execution.

## Storage Isolation

Each shard has its own database. This means:

- Transactions submitted to shard A are not visible from shard B.
- Block numbers within a shard are independent (shard A may be at block 5 while shard B is at block 12).
- State reads (storage, nonces, classes) reflect only the transactions executed in that shard.

## Block Environment

All shards share a single block environment (`BlockEnv`) that contains the current block number, timestamp, gas prices, sequencer address, and Starknet protocol version. Rather than generating these values locally, the shard node derives them from a base chain so that execution uses realistic and up-to-date parameters.

### Block Context Listener

A background task polls the base chain and keeps the shared block environment current. It repeats the following cycle:

1. **Sleep** for the configured poll interval.
2. **Fetch** the latest block from the base chain via its RPC endpoint.
3. **Extract** the block environment fields (block number, timestamp, gas prices, sequencer address, protocol version).
4. **Update** the shared block environment that all workers read from.

Before the listener fetches its first block, the block environment is initialized from the chain's genesis configuration.

The update is an atomic write: workers always see a consistent block environment, never a partially updated one. Workers snapshot the block environment at the start of each execution cycle, so an update mid-cycle does not affect the current batch.

If the base chain RPC call fails (network error, timeout, etc.), the listener logs a warning and retries on the next poll interval. It does not crash or panic. The most recently fetched block environment remains in effect until a successful fetch replaces it.

| Parameter | Default | Description |
|---|---|---|
| `base_chain_url` | (required) | RPC URL of the base chain to poll |
| `block_poll_interval` | 12 seconds | How often to fetch new blocks |

The poll interval should be tuned to match the base chain's block time (e.g., ~12s for Ethereum, ~30s for Starknet). The listener runs as a graceful-shutdown task and exits when the node's task manager signals shutdown.
