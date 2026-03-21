# Static Files Storage

## Overview

Static files are a flat-file storage layer for immutable, append-only block and transaction
data. Heavy values (headers, transactions, receipts, execution traces, state updates) are
stored in sequential `.dat` files instead of MDBX B-trees, while MDBX retains the role of
**authoritative index** — storing pointers into the static files and all mutable/random-access
data.

This design reduces MDBX write amplification for large values (B-tree page splits are
eliminated for data that is only ever appended) and keeps the MDBX file smaller, improving
OS page cache hit rates for the mutable state tables that remain in MDBX.

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                          MDBX (Authority)                          │
│                                                                     │
│  Headers(BlockNumber)        → StaticFileRef { offset, length }     │
│  BlockStateUpdates(BlockNum) → StaticFileRef { offset, length }     │
│  Transactions(TxNumber)      → StaticFileRef { offset, length }     │
│  Receipts(TxNumber)          → StaticFileRef { offset, length }     │
│  TxTraces(TxNumber)          → StaticFileRef { offset, length }     │
│                                                                     │
│  BlockBodyIndices(BlockNum)  → StoredBlockBodyIndices  (direct)     │
│  BlockNumbers(BlockHash)     → BlockNumber             (direct)     │
│  BlockStatusses(BlockNum)    → FinalityStatus          (direct)     │
│  TxNumbers(TxHash)           → TxNumber                (direct)     │
│  BlockHashes(BlockNum)       → BlockHash     (fork mode fallback)   │
│  TxHashes(TxNumber)          → TxHash        (fork mode fallback)   │
│  TxBlocks(TxNumber)          → BlockNumber   (fork mode fallback)   │
│                                                                     │
│  + all mutable state/history/trie/class tables (unchanged)          │
└──────────────────────────┬──────────────────────────────────────────┘
                           │ StaticFileRef pointers
                           ▼
┌─────────────────────────────────────────────────────────────────────┐
│                     Static Files (Data Store)                       │
│                                                                     │
│  blocks/                                                            │
│    headers.dat              Variable-size, offset from MDBX pointer │
│    block_state_updates.dat  Variable-size, offset from MDBX pointer │
│    block_hashes.dat         Fixed 32B per entry, key = block_number │
│                                                                     │
│  transactions/                                                      │
│    transactions.dat         Variable-size, offset from MDBX pointer │
│    receipts.dat             Variable-size, offset from MDBX pointer │
│    tx_traces.dat            Variable-size, offset from MDBX pointer │
│    tx_hashes.dat            Fixed 32B per entry, key = tx_number    │
│    tx_blocks.dat            Fixed 8B per entry, key = tx_number     │
└─────────────────────────────────────────────────────────────────────┘
```

### StaticFileRef

MDBX tables that point to static files store a `StaticFileRef` enum as their value:

```rust
enum StaticFileRef {
    StaticFile { offset: u64, length: u32 },  // 13 bytes: tag(1) + offset(8) + length(4)
    Inline(Vec<u8>),                           // 1 + N bytes: tag(1) + compressed data
}
```

- **`StaticFile`**: Used in sequential (production) mode. Points to a byte range in the
  corresponding `.dat` file. The compressed data at that offset is identical to what MDBX
  would have stored directly in older versions.

- **`Inline`**: Used in fork mode where block numbers are non-sequential and static file
  appends are not possible. The compressed data is stored directly in the MDBX value, similar
  to the pre-static-files behavior.

### Why some values stay in MDBX

| Table | Why it stays in MDBX |
|---|---|
| `BlockBodyIndices` | Too small (~10 bytes). The 13-byte pointer would be larger than the data. |
| `BlockNumbers` | Random-key reverse index (keyed by hash). Cannot be sequentially appended. |
| `TxNumbers` | Random-key reverse index (keyed by hash). Cannot be sequentially appended. |
| `BlockStatusses` | Mutable (finality status changes from AcceptedOnL2 to AcceptedOnL1). |
| `BlockHashes`, `TxHashes`, `TxBlocks` | Also in static files for sequential mode, but kept in MDBX as fallback for fork mode where static files are not written. |

## MDBX as the Sole Authority

**Critical invariant**: MDBX is the single source of truth for what data exists.

- A reader opens an MDBX read transaction, which provides a consistent snapshot.
- All reads go through MDBX first: if the MDBX entry doesn't exist, the data doesn't exist.
- For `StaticFileRef::StaticFile` pointers, the reader fetches data from the `.dat` file
  at the specified offset. This data is guaranteed to be on disk because it was written
  *before* the MDBX transaction that committed the pointer.
- Static files have no independent index or manifest. They are raw data blobs addressed
  solely by MDBX pointers.

This means:
- **No stale reads**: A reader's MDBX snapshot determines exactly which static file data
  is visible. Even if a concurrent writer has appended new data to the static files, the
  reader won't see it because the MDBX pointers aren't in its snapshot.
- **No phantom reads**: If an MDBX pointer exists, the referenced data is guaranteed to
  be durable in the static file (written and flushed before the MDBX commit).

## Write Path

### Sequential mode (production)

Used when block numbers are sequential starting from the current static file count:

```
1. Append compressed data to .dat files (buffered pwrite, no fsync)
   → returns (offset, length) for each variable-size entry
2. Append fixed-size entries (block_hash, tx_hash, tx_block) to their .dat files
3. Write MDBX entries:
   - StaticFileRef::pointer(offset, length) for each variable-size table
   - Direct values for BlockBodyIndices, BlockNumbers, BlockStatusses, TxNumbers
4. MDBX commit (caller calls provider.commit())
   - Static files remap (lightweight, updates mmap pointers for subsequent readers)
```

### Non-sequential mode (fork)

Used when block numbers don't match the expected next static file position (e.g., fork
provider inserting blocks at arbitrary positions):

```
1. Compress data in memory
2. Write MDBX entries:
   - StaticFileRef::inline(compressed_bytes) for each variable-size table
   - Direct values for all other tables
   - BlockHashes, TxHashes, TxBlocks also written to MDBX (no static file equivalent)
3. MDBX commit
```

### Batch mode (sync pipeline)

`insert_block_data_batch()` provides a two-phase write optimized for inserting many
blocks in a single MDBX transaction:

```
Phase 1 — Static file appends (sequential I/O):
  Pre-size write buffers based on batch dimensions.
  For each block: append header, state_update, block_hash to static files.
  For each tx: append transaction, receipt, trace, tx_hash, tx_block.
  Collect all (offset, length) pointers in memory.

Phase 2 — MDBX writes (B-tree inserts):
  For each block: put Headers, BlockStateUpdates, BlockBodyIndices, BlockNumbers, etc.
  For each tx: put Transactions, Receipts, TxTraces, TxNumbers.
  For each block: put class artifacts, declarations.

Commit: single MDBX transaction for the entire batch.
```

This separates sequential file I/O from random B-tree inserts, improving disk locality
and reducing MDBX transaction overhead (one tx instead of one per block).

## Read Path

All reads follow the same pattern:

1. **MDBX lookup**: Read the table entry using the MDBX transaction snapshot.
   - If not found → data doesn't exist, return `None`.
2. **Resolve `StaticFileRef`**:
   - `StaticFile { offset, length }` → read from the `.dat` file via mmap (fast path)
     or pread (fallback for recently-written data not yet remapped).
   - `Inline(bytes)` → decompress directly from the MDBX value.
3. **Decompress**: Apply the same `Decompress` codec as the pre-static-files implementation.

For fixed-size static file data (block_hashes, tx_hashes, tx_blocks), reads go directly
to the static file using `key * record_size` as the offset. These reads are gated by the
existence of a corresponding MDBX pointer entry (e.g., if `Headers(5)` exists, then
`block_hashes[5]` is guaranteed to exist in the static file).

If the static file read returns `None` (e.g., fork mode where static files weren't written),
the read falls back to the MDBX table (BlockHashes, TxHashes, TxBlocks).

## Crash Recovery

### Why recovery is needed

Static file appends happen **before** the MDBX transaction commits. This ordering is
required so that data is durable on disk before MDBX makes the pointers visible. But it
creates a window where a crash leaves orphaned data:

```
Timeline:
  t0: Append block data to headers.dat        ← data on disk (or in OS page cache)
  t1: Append tx data to transactions.dat      ← data on disk
  t2: Write MDBX pointer entries              ← in uncommitted MDBX transaction
  t3: --- CRASH ---
  t4: MDBX rolls back (automatic, ACID)       ← pointers never committed
      Static files still have data from t0-t1  ← ORPHANED
```

After restart:
- MDBX is clean (rolled back to last committed state).
- Static files have extra data at the tail that MDBX doesn't know about.
- `FileStore::open()` reads the actual file length, so `cached_len` includes the orphaned data.
- The next `append()` call would write at `cached_len` (past the orphaned data), producing
  an offset that doesn't match the expected position. For fixed-size columns, the
  `debug_assert!(expected_offset == actual_offset)` would fire.

### How recovery works

On every database open (`Db::new()`, `Db::open()`, `Db::open_no_sync()`), the
`recover_static_files()` function runs:

1. Open an MDBX read transaction.
2. For each variable-size column (headers, state_updates, transactions, receipts, traces):
   - Read the **last** entry from the corresponding MDBX table using a cursor.
   - Extract `offset + length` from the `StaticFileRef::StaticFile` pointer.
   - Truncate the `.dat` file to `offset + length` bytes.
   - If the MDBX table is empty, truncate to 0.
3. For each fixed-size column (block_hashes, tx_hashes, tx_blocks):
   - Determine the committed count from the last MDBX entry's key + 1.
   - Truncate to `count * record_size` bytes.

After truncation, `cached_len` matches the file length, and the next `append()` writes
at the correct position.

### What recovery handles

| Crash scenario | Static file state | MDBX state | Recovery action |
|---|---|---|---|
| During phase 1 (static file appends) | Partial data at tail | No uncommitted entries | Truncate to MDBX state (0 or previous commit) |
| During phase 2 (MDBX puts) | Complete data at tail | Uncommitted tx | Truncate tail (MDBX rolled back) |
| During MDBX commit | Complete data at tail | Committed or rolled back | If committed: no-op. If rolled back: truncate tail. |
| After MDBX commit, before remap | Complete data, stale mmap | Committed | No-op (data is consistent, remap happens on next open) |
| Clean shutdown | Consistent | Consistent | No-op (truncation is idempotent) |

### What recovery does NOT handle

- **MDBX corruption**: If MDBX itself is corrupted (disk failure, etc.), recovery cannot
  proceed because it depends on reading MDBX state. This is an MDBX-level concern.
- **Static file corruption within committed range**: If a disk failure corrupts bytes
  within the committed region of a `.dat` file, the data is silently corrupted. Recovery
  only truncates the tail; it does not verify checksums within the committed range.
  (This is the same as MDBX — neither system checksums individual values.)
- **Bit rot / silent data corruption**: Neither MDBX nor static files include per-record
  checksums. Filesystem-level integrity (ZFS, Btrfs) is recommended for production.

## FileStore Implementation

### I/O strategy

- **Reads**: Memory-mapped (`mmap`) for data within the mapped region. Zero-copy from the
  kernel page cache — no syscall overhead. Falls back to `pread` for recently-appended data
  not yet covered by the mmap.

- **Writes**: Buffered `pwrite`. Small appends accumulate in an in-memory buffer (64KB
  initial, auto-flush at 256KB). Flushed to disk as a single `pwrite` on `remap()` or
  `sync()`. This reduces syscall count from ~54 per block (10 txs) to a handful per batch.

- **No per-write fsync**: Static files rely on MDBX's durability model. On crash, orphaned
  data is truncated by recovery. Explicit `sync()` is available for callers that need
  stronger durability guarantees (e.g., before checkpoints).

### Concurrency

- `pread` is thread-safe (no shared file offset), so reads don't need a lock.
- `mmap` reads use a `RwLock` (shared access for reads, exclusive for remap).
- Writes use a `Mutex` to serialize appends and protect the write buffer.
- `cached_len` is an `AtomicU64` to avoid `fstat` syscalls on every access.

### Remap

After writes, the mmap doesn't cover the new data (it was created with the old file length).
`remap()` flushes the write buffer and creates a new mmap covering all data. This is called
automatically on `commit()` so subsequent readers see the new data through mmap.

## Assumptions

1. **Blocks are inserted sequentially in production mode.** Block numbers must equal the
   current static file entry count. Non-sequential insertion (fork mode) falls back to
   inline MDBX storage.

2. **Blocks are never deleted or modified after insertion.** The static files are
   append-only. There is no mechanism to update or remove individual entries. Reorgs are
   not supported by this storage layer.

3. **Single writer at a time.** Only one MDBX write transaction can be active, and static
   file appends are serialized by the write lock. Concurrent readers are safe.

4. **Recovery runs before any writes.** The `recover_static_files()` call in every `Db`
   constructor ensures orphaned data is truncated before the first append.

5. **MDBX is always openable after a crash.** MDBX's ACID properties guarantee this.
   Static file recovery depends on being able to read the MDBX state.

6. **File system preserves write ordering within a file.** `pwrite` to a file followed by
   another `pwrite` to the same file is expected to be visible in order. This is guaranteed
   by POSIX and all major file systems.

7. **Inline mode produces identical data.** `StaticFileRef::Inline` stores the same
   compressed bytes that `StaticFileRef::StaticFile` would reference in the `.dat` file.
   Readers use the same `Decompress` codec for both cases.

## Directory Layout

```
db/
├── mdbx.dat              MDBX database (pointers + indexes + mutable state)
├── mdbx.lck              MDBX lock file
├── db.version            Database version (currently 10)
└── static/
    ├── blocks/
    │   ├── headers.dat              Variable-size, compressed VersionedHeader
    │   ├── block_hashes.dat         Fixed 32 bytes per entry
    │   └── block_state_updates.dat  Variable-size, compressed StateUpdateEnvelope
    └── transactions/
        ├── transactions.dat         Variable-size, compressed TxEnvelope
        ├── receipts.dat             Variable-size, compressed ReceiptEnvelope
        ├── tx_traces.dat            Variable-size, compressed TypedTransactionExecutionInfo
        ├── tx_hashes.dat            Fixed 32 bytes per entry
        └── tx_blocks.dat            Fixed 8 bytes per entry
```
