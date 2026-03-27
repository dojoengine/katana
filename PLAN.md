# Versioned Starknet RPC Implementation Plan

## Overview

Implement versioned Starknet RPC API support in Katana, exposing spec versions 0.9.0 and 0.10.0 via URL path prefixes (`/rpc/v0_9`, `/rpc/v0_10`). The default `/` path routes to v0.9 (current). All three API groups (Read, Write, Trace) are versioned. Non-Starknet APIs (Katana, Dev, TxPool, etc.) are available on all paths.

## Goals

- Expose v0.9.0 and v0.10.0 Starknet APIs at distinct URL paths
- Keep RPC handlers clean — no version branching in handler logic
- Version-specific types defined at the trait level via the `#[rpc]` macro
- Default `/` routes to v0.9 (current); `/rpc/v0_10` opts in to latest

## Non-Goals

- WebSocket subscription API versioning (out of scope)
- Supporting spec versions older than 0.9.0
- Runtime-configurable version selection (compile-time trait structure)

## Assumptions and Constraints

- jsonrpsee 0.26 has no built-in path-based routing; we use a tower middleware layer
- v0.9 and v0.10 have identical method sets (no methods added/removed)
- Only 4 types differ between versions (BlockHeader, EmittedEvent, StateDiff, PreConfirmedStateUpdate)
- `RpcModule::merge` rejects duplicate method names, so each version needs its own `RpcModule`

## Technical Design

### Type Differences (v0.9 → v0.10)

| Type | v0.9 | v0.10 |
|------|------|-------|
| `BlockHeader` | Current fields | +7 fields: `event_commitment`, `event_count`, `receipt_commitment`, `state_diff_commitment`, `state_diff_length`, `transaction_commitment`, `transaction_count` |
| `EmittedEvent` | `event_index`/`transaction_index` are `Option` (skipped if None) | Both become **required** (always serialized) |
| `StateDiff` | `migrated_compiled_classes` is `Option` (skipped if None) | `migrated_compiled_classes` is **required** (defaults to empty) |
| `PreConfirmedStateUpdate` | `old_root` required | `old_root` becomes optional |

### Architecture

```
Request → Tower Middleware (path inspection)
           ├─ /rpc/v0_9  → v0.9 RpcModule (starknet + shared modules)
           ├─ /rpc/v0_10 → v0.10 RpcModule (starknet + shared modules)
           └─ /          → v0.9 RpcModule (default)
```

### Module Structure

```
crates/rpc/rpc-api/src/
├── starknet.rs          → starknet/mod.rs (shared imports, version enum)
│                          starknet/v0_9.rs (#[rpc] traits with v0.9 types)
│                          starknet/v0_10.rs (#[rpc] traits with v0.10 types)

crates/rpc/rpc-types/src/
├── block.rs             (existing shared types, kept as-is for internal use)
├── event.rs             (existing shared types)
├── state_update.rs      (existing shared types)
├── v0_9/
│   ├── mod.rs
│   ├── block.rs         (BlockHeader WITHOUT 7 new fields)
│   ├── event.rs         (EmittedEvent with Option event_index/transaction_index, skip_serializing_if)
│   └── state_update.rs  (StateDiff with Option migrated_compiled_classes, skip_serializing_if)
├── v0_10/
│   ├── mod.rs
│   ├── block.rs         (BlockHeader WITH 7 new fields)
│   ├── event.rs         (EmittedEvent with required event_index/transaction_index)
│   └── state_update.rs  (StateDiff with required migrated_compiled_classes)

crates/rpc/rpc-server/src/
├── starknet/
│   ├── mod.rs           (shared StarknetApi struct + internal helpers, unchanged)
│   ├── read.rs          → read/mod.rs + read/v0_9.rs + read/v0_10.rs
│   ├── write.rs         → write/mod.rs + write/v0_9.rs + write/v0_10.rs
│   ├── trace.rs         → trace/mod.rs + trace/v0_9.rs + trace/v0_10.rs
├── versioned.rs         (VersionedRpcModule builder + tower middleware)
├── lib.rs               (RpcServer updated to accept versioned modules)
```

---

## Implementation Plan

### Serial Dependencies (Must Complete First)

#### Phase 0: Versioned RPC Types
**Prerequisite for:** All subsequent phases

| Task | Description | Output |
|------|-------------|--------|
| 0.1 | Create `crates/rpc/rpc-types/src/v0_9/` module with version-specific block, event, and state_update types. These wrap/re-export shared types but serialize according to v0.9 rules (skip optional new fields). | `v0_9::BlockHeader`, `v0_9::EmittedEvent`, `v0_9::StateDiff`, `v0_9::StateUpdate` and their response wrappers |
| 0.2 | Create `crates/rpc/rpc-types/src/v0_10/` module with version-specific types. BlockHeader includes the 7 new commitment/count fields. EmittedEvent has required `event_index`/`transaction_index`. StateDiff has required `migrated_compiled_classes`. | `v0_10::BlockHeader`, `v0_10::EmittedEvent`, `v0_10::StateDiff`, `v0_10::StateUpdate` and their response wrappers |
| 0.3 | Add `From` conversions from internal/shared types to each version's types. The internal helpers return shared types; the trait impls convert to version-specific types via `.into()`. | `From<shared::X> for v0_9::X` and `From<shared::X> for v0_10::X` |
| 0.4 | Add v0.10 test fixtures for blocks and events (v0.10/blocks/, v0.10/events/) alongside the existing v0.10/state-updates/. Add roundtrip serde tests for all new versioned types. | Test fixtures + passing serde tests |

---

### Parallel Workstreams

#### Workstream A: Versioned API Traits (`rpc-api`)
**Dependencies:** Phase 0
**Can parallelize with:** Workstream B, C

| Task | Description | Output |
|------|-------------|--------|
| A.1 | Convert `crates/rpc/rpc-api/src/starknet.rs` into a `starknet/` module directory. Create `starknet/mod.rs` with shared imports and a version constant per version. | `starknet/mod.rs` with `V0_9_SPEC_VERSION = "0.9.0"` and `V0_10_SPEC_VERSION = "0.10.0"` |
| A.2 | Create `starknet/v0_9.rs` with `#[rpc(server, namespace = "starknet")]` traits: `StarknetApi`, `StarknetWriteApi`, `StarknetTraceApi`. These use `katana_rpc_types::v0_9::*` response types for the 4 affected methods. `specVersion` returns `"0.9.0"`. Unaffected methods use shared types. | Three `*Server` traits generated by jsonrpsee |
| A.3 | Create `starknet/v0_10.rs` with identical structure but using `katana_rpc_types::v0_10::*` response types. `specVersion` returns `"0.10.0"`. | Three `*Server` traits for v0.10 |
| A.4 | Update `crates/rpc/rpc-api/src/lib.rs` to expose versioned submodules: `pub mod starknet { pub mod v0_9; pub mod v0_10; }`. Remove the old `starknet.rs`. | Updated module structure |

#### Workstream B: Versioned Server Impls (`rpc-server`)
**Dependencies:** Phase 0, Workstream A
**Can parallelize with:** Workstream C (partially)

| Task | Description | Output |
|------|-------------|--------|
| B.1 | Implement `v0_9::StarknetApiServer` for `StarknetApi<...>` in `read/v0_9.rs`. Each method calls the existing shared helper (e.g., `self.block_with_tx_hashes()`), then converts the result to v0.9 types via `.into()`. | v0.9 Read API impl |
| B.2 | Implement `v0_10::StarknetApiServer` for `StarknetApi<...>` in `read/v0_10.rs`. Same pattern, converting to v0.10 types. | v0.10 Read API impl |
| B.3 | Implement versioned Write API (`write/v0_9.rs`, `write/v0_10.rs`). Since Write API types are identical, these are thin wrappers delegating to shared helpers. | v0.9 + v0.10 Write API impls |
| B.4 | Implement versioned Trace API (`trace/v0_9.rs`, `trace/v0_10.rs`). Same as Write — identical types, thin delegation. | v0.9 + v0.10 Trace API impls |
| B.5 | Remove old `read.rs`, `write.rs`, `trace.rs` single-version impls. Update `mod.rs` to export versioned submodules. | Clean module structure |

#### Workstream C: Path-Based Routing Middleware
**Dependencies:** None (can start immediately, tested with mock modules)
**Can parallelize with:** Workstreams A, B

| Task | Description | Output |
|------|-------------|--------|
| C.1 | Create `crates/rpc/rpc-server/src/versioned.rs` with a `VersionedRpcModule` struct that holds a map of `path_prefix → Methods` and a default `Methods`. | `VersionedRpcModule` struct |
| C.2 | Implement a tower `Layer`/`Service` (`VersionedRpcRouter`) that inspects `req.uri().path()`, strips the version prefix, and forwards to the appropriate jsonrpsee `Methods` set. For unrecognized paths, fall through to default. | `VersionedRpcRouterLayer` + `VersionedRpcRouterService` |
| C.3 | Update `RpcServer` to accept versioned module configuration. Add a `.versioned_modules(VersionedRpcModule)` builder method alongside the existing `.module()`. Wire the tower layer into `start()`. | Updated `RpcServer::start()` |
| C.4 | Write integration test: start server with two versioned modules, verify that `/rpc/v0_9` returns `specVersion = "0.9.0"`, `/rpc/v0_10` returns `"0.10.0"`, and `/` returns `"0.9.0"`. | Passing integration test |

---

### Merge Phase

#### Phase N: Integration & Wiring
**Dependencies:** Workstreams A, B, C

| Task | Description | Output |
|------|-------------|--------|
| N.1 | Update `crates/node/sequencer/src/lib.rs` module assembly: build two `RpcModule`s (v0.9, v0.10) by calling `v0_9::StarknetApiServer::into_rpc()` and `v0_10::StarknetApiServer::into_rpc()` on the same `starknet_api` instance. Merge shared modules (Katana, Dev, TxPool, etc.) into both. Pass both to `RpcServer` via `VersionedRpcModule`. | Versioned server startup |
| N.2 | Update any RPC client code or test utilities that import from `katana_rpc_api::starknet::*` to use the versioned paths (e.g., `katana_rpc_api::starknet::v0_9::*`). | All imports updated |
| N.3 | End-to-end test: start Katana, hit `/rpc/v0_9/` and `/rpc/v0_10/` with `starknet_getBlockWithTxHashes`, verify the v0.10 response includes the 7 new block header fields and v0.9 does not. | Passing E2E test |
| N.4 | Run full test suite (`cargo nextest run`), fix any regressions. Run clippy and fmt. | Green CI |

---

## Testing and Validation

- **Unit tests**: Serde roundtrip for all versioned types (v0_9 and v0_10 block, event, state_update)
- **Integration tests**: Path-based routing (C.4), spec version per path
- **E2E tests**: Full Katana startup with versioned endpoints (N.3)
- **Regression**: Full `cargo nextest run` to catch import/type breakage

## Verification Checklist

```bash
# Build
cargo build

# Unit tests for versioned types
cargo nextest run -p katana-rpc-types

# Integration tests for routing
cargo nextest run -p katana-rpc-server

# Full test suite
cargo nextest run

# Lint
./scripts/clippy.sh
cargo +nightly-2025-02-20 fmt --all --check
```

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| jsonrpsee tower middleware can't intercept path before RPC dispatch | Medium | High | Prototype C.1-C.2 early; fall back to `to_service_builder()` if middleware approach doesn't work |
| Large number of trait impls creates maintenance burden | Low | Medium | Write/Trace impls are thin wrappers; only Read has real version-specific logic (4 methods) |
| Breaking existing client imports (`katana_rpc_api::starknet::*`) | High | Low | Systematic search-and-replace in N.2; compiler will catch all misses |
| Health check proxy (`/`) conflicts with default RPC routing | Medium | Medium | Ensure health check layer runs before versioned router; test GET `/` still returns health |

## Open Questions

- [ ] Should the v0.10 block header commitment fields be computed from actual data, or zero-filled initially? (Likely needs executor/provider changes to populate them)
- [ ] Should the version path format be `/rpc/v0_9` or `/rpc/v0.9` or `/v0_9`? (Using `/rpc/v0_9` as proposed)

## Decision Log

| Decision | Rationale | Alternatives Considered |
|----------|-----------|------------------------|
| Version all three API groups (Read, Write, Trace) | Consistency; future-proof for when Write/Trace diverge | Only version Read API |
| Default `/` → v0.9 | Avoid breaking existing clients; opt-in to v0.10 | Default to latest (v0.10) |
| Tower middleware for routing | Less invasive than custom accept loop; keeps `Server::start()` flow | `to_service_builder()` custom accept loop |
| Separate versioned types (not serde conditional) | Clean separation at macro level; handlers stay version-unaware | `#[serde(skip_serializing_if)]` with runtime version flag |
| Non-Starknet APIs on all paths | Clients using versioned paths shouldn't lose access to dev/katana APIs | Restrict to `/` only |
