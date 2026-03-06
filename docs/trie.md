# Merkle Patricia Trie

Katana uses a Binary Merkle-Patricia Trie (MPT) to compute and verify Starknet state commitments. The implementation is ported from [pathfinder](https://github.com/eqlabs/pathfinder)'s merkle-tree crate and lives across three layers:

- **`katana-trie`** — Core trie engine (pure computation, no DB dependency)
- **`katana-db`** — Database-backed storage, persistence, and loading
- **`katana-provider`** — Integrates trie updates into the block production pipeline

## Architecture

```
┌──────────────────────────────────────────────────┐
│               Provider (TrieWriter)              │
│  Orchestrates: load → compute → persist          │
└──────────────┬───────────────────────────────────┘
               │
       ┌───────┴───────┐
       │               │
┌──────▼──────┐  ┌─────▼──────────────┐
│ katana-trie │  │     katana-db      │
│             │  │                    │
│ MerkleTree  │  │ TrieDbFactory      │
│ Storage     │  │ DbTrieStorage      │
│ MemStorage  │  │ load_trie_to_memory│
│ ProofNode   │  │ persist_trie_update│
└─────────────┘  └────────────────────┘
```

## Three Starknet Tries

Starknet's state commitment is derived from three tries:

| Trie | Hash | Height | Key | Value |
|------|------|--------|-----|-------|
| **Classes** | Poseidon | 251 | Class hash | `Poseidon("CONTRACT_CLASS_LEAF_V0", compiled_hash)` |
| **Contracts** | Pedersen | 251 | Contract address | `Pedersen(Pedersen(Pedersen(class_hash, storage_root), nonce), 0)` |
| **Storage** (per-contract) | Pedersen | 251 | Storage key | Storage value |

Each is wrapped in a typed struct (`ClassesTrie`, `ContractsTrie`, `StoragesTrie`) that handles key/value encoding and delegates to the generic `MerkleTree<H, HEIGHT>`.

## Core Engine (`katana-trie`)

### Node Types

The trie uses four node representations across different lifecycle phases:

**In-memory during mutation** (`InternalNode`):
```
Unresolved(TrieNodeIndex)  — placeholder, not yet loaded from storage
Binary { left, right }     — branch with two Rc<RefCell<InternalNode>> children
Edge { path, child }       — path-compressed node
Leaf(Felt)                 — terminal node carrying its hash value
```

**Persisted in DB** (`StoredNode`):
```
Binary { left, right }            — children referenced by TrieNodeIndex
Edge { child, path }              — child referenced by TrieNodeIndex
LeafBinary { left_hash, right_hash } — both children are leaves, hashes embedded
LeafEdge { path, child_hash }        — child is a leaf, hash embedded
```

The `LeafBinary` and `LeafEdge` variants embed child hashes directly in the parent node. This eliminates the need for separate leaf tables and ensures correct historical trie access — leaf data is versioned together with the tree structure.

**During commit** (`Node`):
Same structure as `StoredNode`, but children are referenced by `NodeRef` — either a `StorageIndex` (existing node in storage) or an `Index` (position in `TrieUpdate::nodes_added`). This allows the commit to produce a self-contained update without allocating storage indices.

### MerkleTree\<H, HEIGHT\>

The core trie implementation, generic over hash function and tree height.

**Key operations:**

- **`set(storage, key, value)`** — Insert or update. Traverses from root, resolving `Unresolved` nodes lazily from storage. When the key diverges from an existing edge, splits the edge into a binary node + two edges. Setting value to `ZERO` deletes the leaf.

- **`commit(storage) → TrieUpdate`** — Walks the mutated tree bottom-up, hashing every node. Returns a `TrieUpdate` containing:
  - `nodes_added` — new `(hash, Node)` pairs in topological order (root is last)
  - `nodes_removed` — indices of nodes that were replaced
  - `root_commitment` — the final root hash

- **`get_proofs(root, storage, keys) → Vec<Vec<(ProofNode, Felt)>>`** — Generates Merkle proofs for a set of keys against a committed tree. Used for RPC `getProof` responses.

**Lazy resolution:** During `set()`, the tree only resolves nodes along the traversal path. Unvisited subtrees remain as `Unresolved` placeholders. On `commit()`, unresolved nodes contribute their pre-existing hash without being loaded — only the mutated path is re-hashed.

### Storage Trait

```rust
pub trait Storage {
    fn get(&self, index: TrieNodeIndex) -> Result<Option<StoredNode>>;
    fn hash(&self, index: TrieNodeIndex) -> Result<Option<Felt>>;
}
```

This is the only interface the trie engine needs from its backing store. Two implementations exist:

- **`MemStorage`** — HashMap-backed, used for in-memory computation and tests
- **`DbTrieStorage`** — Database-backed with LRU cache, used for read-only operations (proof generation, root queries)

### MemStorage

HashMap-based storage that can hold nodes at arbitrary indices:

```rust
pub struct MemStorage {
    nodes: HashMap<u64, (Felt, StoredNode)>,
    next_index: u64,
}
```

Two usage patterns:
1. **Fresh computation** — Start empty, call `apply_update()` after each `commit()` to materialize nodes with sequential indices.
2. **Loaded from DB** — Populated via `insert_node(index, hash, node)` with original DB indices, then used as storage for further mutations.

## Database Layer (`katana-db`)

### Tables

Five tables store all trie data:

| Table | Key | Value | Purpose |
|-------|-----|-------|---------|
| `TrieClassNodes` | `u64` | `TrieNodeEntry` | Class trie nodes |
| `TrieContractNodes` | `u64` | `TrieNodeEntry` | Contracts trie nodes |
| `TrieStorageNodes` | `u64` | `TrieNodeEntry` | Storage trie nodes (all contracts share one table) |
| `TrieRoots` | `u64` | `u64` | Maps (trie_type, block) → root node index |
| `TrieBlockLog` | `u64` | `BlockList` | Maps (trie_type, block) → added node indices (for revert) |

`TrieNodeEntry` combines a `StoredNode` with its hash. The key for `TrieRoots` and `TrieBlockLog` is a composite of trie type (upper 8 bits) and block number (lower 56 bits). Storage trie roots use a separate key space (high bit set) with an address-derived hash.

### Persistence Flow

```
persist_trie_update(tx, update, block, trie_type, next_index):
  for each (hash, node) in update.nodes_added:
    resolve NodeRef → TrieNodeIndex using base offset
    write TrieNodeEntry to node table at next_index++
  write root index to TrieRoots
  write added indices to TrieBlockLog
```

Node indices are monotonically increasing per table. New nodes from each block are appended after existing ones. The root is always the last node added.

### Loading and Factory

`TrieDbFactory` creates trie instances from the database. It offers two modes:

**DB-backed (for reads/proofs):**
```rust
factory.classes_trie(block)     → ClassesTrie<DbTrieStorage<...>>
factory.contracts_trie(block)   → ContractsTrie<DbTrieStorage<...>>
factory.storages_trie(addr, block) → StoragesTrie<DbTrieStorage<...>>
```
Nodes are resolved lazily from DB via `DbTrieStorage` (with LRU cache). Used for proof generation and root hash queries where loading the entire trie would be wasteful.

**In-memory (for mutation):**
```rust
factory.classes_trie_in_memory(block)     → ClassesTrie<MemStorage>
factory.contracts_trie_in_memory(block)   → ContractsTrie<MemStorage>
factory.storages_trie_in_memory(addr, block) → StoragesTrie<MemStorage>
```
All reachable nodes are loaded from DB into a `MemStorage` via BFS (`load_trie_to_memory`), then the trie operates purely in memory. Used for block production where multiple `set()` calls would otherwise trigger repeated DB reads.

### Revert and Pruning

- **`revert_trie_to_block(tx, target, latest)`** — For each block after target, reads `TrieBlockLog` to find added node indices, deletes those nodes, then deletes the root and log entries. Restores the trie to its state at the target block.

- **`prune_trie_block(tx, block)`** — Removes root and log entries for a block without deleting nodes (which may be shared with later blocks). Used by the sync pipeline to bound metadata growth.

## Provider Integration

The `TrieWriter` trait is implemented by `DbProvider` and orchestrates the full update cycle:

### `trie_insert_declared_classes(block, updates)`

```
1. Load classes trie from previous block into memory
2. For each (class_hash, compiled_hash): trie.insert(...)
3. trie.commit() → TrieUpdate
4. persist_trie_update() → write to DB
5. Return root commitment
```

### `trie_insert_contract_updates(block, state_updates)`

```
1. For each contract with storage changes:
   a. Load its storage trie into memory
   b. Insert all storage key/value updates
   c. Commit and persist → get storage_root
2. Collect nonce and class_hash updates per contract
3. Compute contract state hash for each updated contract:
   Pedersen(Pedersen(Pedersen(class_hash, storage_root), nonce), 0)
4. Load contracts trie into memory
5. Insert all (address, state_hash) pairs
6. Commit and persist → return root commitment
```

## Hash Functions

- **Classes trie:** Poseidon hash (as specified by Starknet for class commitments)
- **Contracts trie:** Pedersen hash
- **Storage tries:** Pedersen hash
- **Edge node hash:** `H(child_hash, path_felt) + path_length`
- **Binary node hash:** `H(left_hash, right_hash)`

## Key Design Decisions

1. **Leaf hashes embedded in parent nodes** — `LeafBinary` and `LeafEdge` variants store child hashes directly rather than referencing separate leaf entries. This ensures historical correctness (leaf values are versioned with the tree structure) and simplifies the storage model.

2. **In-memory-first mutation** — Trie mutations during block production load the entire existing trie into memory before applying updates. This decouples computation from DB access, enables pure in-memory tree operations, and opens the door for background computation without holding DB transactions.

3. **Lazy resolution for reads** — DB-backed tries used for proof generation resolve nodes on demand with an LRU cache. This avoids loading entire tries when only a small traversal path is needed.

4. **Append-only node storage** — New nodes get monotonically increasing indices. Combined with `TrieBlockLog`, this enables efficient revert by deleting exactly the nodes added by reverted blocks.

5. **Shared storage trie table** — All per-contract storage tries share a single `TrieStorageNodes` table (with address-derived root keys) rather than having per-contract tables. This keeps the table count fixed regardless of how many contracts exist.
