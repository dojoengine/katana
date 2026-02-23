---
name: implement-rpc-api
description: How to implement a new JSON-RPC API in this codebase — defining the API trait, types, error enum, and server handler.
---

# Implement a New JSON-RPC API

This skill describes how to add a new JSON-RPC API namespace to Katana. There are five steps:

1. Define the API trait (`katana-rpc-api`)
2. Define custom types, if needed (`katana-rpc-types`)
3. Define a dedicated error enum (`katana-rpc-api`)
4. Implement the server handler (`katana-rpc-server`)
5. Register the API in the node implementation(s) (`katana-sequencer-node`/`katana-full-node` for sequencer/full node respectively)

The crates involved:

| Crate | Path | Purpose |
|---|---|---|
| `katana-rpc-api` | `crates/rpc/rpc-api/` | API trait definitions and error types |
| `katana-rpc-types` | `crates/rpc/rpc-types/` | RPC request/response types |
| `katana-rpc-server` | `crates/rpc/rpc-server/` | Server-side implementations |
| `katana-sequencer-node` | `crates/node/sequencer/` | Sequencer node — wires RPC modules into the server |
| `katana-full-node` | `crates/node/full/` | Full node — wires RPC modules into the server |
| `katana-node-config` | `crates/node/config/` | Node configuration including `RpcModuleKind` |

Throughout this guide, `<name>` is the API namespace (e.g., `dev`, `tee`, `starknet`).

---

## Step 1: Define the API Trait

Create a new module in `crates/rpc/rpc-api/src/<name>.rs` and define the trait using the `jsonrpsee` proc macro.

### Naming conventions

- **Trait name**: `<Name>Api` — PascalCase of the namespace with `Api` suffix (e.g., `DevApi`, `TeeApi`).
- **Namespace**: The `namespace` attribute in the `#[rpc(...)]` macro must match the JSON-RPC namespace exactly (e.g., `"dev"` produces methods like `dev_generateBlock`).
- **Method names**: Use the `#[method(name = "...")]` attribute with camelCase (e.g., `"generateBlock"`). The Rust function name uses snake_case.

### Template

```rust
use jsonrpsee::core::RpcResult;
use jsonrpsee::proc_macros::rpc;

#[cfg_attr(not(feature = "client"), rpc(server, namespace = "<name>"))]
#[cfg_attr(feature = "client", rpc(client, server, namespace = "<name>"))]
pub trait <Name>Api {
    /// Brief description of what this method does.
    #[method(name = "methodName")]
    async fn method_name(&self, param: ParamType) -> RpcResult<ResponseType>;
}
```

### Key rules

- All methods must be `async`.
- Return type is always `RpcResult<T>` (alias for `Result<T, jsonrpsee::types::ErrorObjectOwned>`).
- The `#[cfg_attr]` pattern enables client code generation only when the `client` feature is active, keeping the server build lighter.
- If a method can have a default implementation (e.g., returning a constant), implement it directly in the trait body. See `StarknetApi::spec_version` for an example.

### Register the module

Add the new module to `crates/rpc/rpc-api/src/lib.rs`:

```rust
pub mod <name>;
```

If the API is feature-gated:

```rust
#[cfg(feature = "<feature>")]
pub mod <name>;
```

### Reference examples

- **Simple API**: `crates/rpc/rpc-api/src/dev.rs` — `DevApi` with straightforward methods.
- **Feature-gated API**: `crates/rpc/rpc-api/src/tee.rs` — `TeeApi` behind the `tee` feature.
- **Split API (read/write/trace)**: `crates/rpc/rpc-api/src/starknet.rs` — Multiple traits sharing the same namespace.

---

## Step 2: Define Custom Types (if needed)

If the API uses request/response types that don't already exist in `katana-primitives` or `katana-rpc-types`, define them in `crates/rpc/rpc-types/src/`.

### Conventions

- All types must derive `Debug`, `Clone`, `Serialize`, `Deserialize`.
- Use `#[serde(rename_all = "camelCase")]` for field names that should be camelCase in JSON.
- Use `#[serde(tag = "type")]` for enum variants that should be discriminated by a `type` field.
- Use `#[serde(flatten)]` to inline nested structs.
- Hex-encoded numeric fields use custom serializers from `serde_utils` (e.g., `serialize_as_hex`, `deserialize_u128`).
- Types that map to internal primitives (`katana-primitives`) should implement `From` conversions.

### Template

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct <Name>Response {
    pub field_one: String,
    pub field_two: u64,
}
```

---

## Step 3: Define a Dedicated Error Enum

Every API namespace must have its own error enum in `crates/rpc/rpc-api/src/error/`. Create `crates/rpc/rpc-api/src/error/<name>.rs`.

### Template

```rust
use jsonrpsee::types::ErrorObjectOwned;

#[derive(thiserror::Error, Clone, Debug)]
pub enum <Name>ApiError {
    #[error("Description of error A")]
    ErrorA,

    #[error("Description of error B: {0}")]
    ErrorB(String),
}

impl From<<Name>ApiError> for ErrorObjectOwned {
    fn from(err: <Name>ApiError) -> Self {
        let code = match &err {
            <Name>ApiError::ErrorA => 1,
            <Name>ApiError::ErrorB(_) => 2,
        };
        ErrorObjectOwned::owned(code, err.to_string(), None::<()>)
    }
}
```

### Key rules

- Use `thiserror::Error` for the enum.
- Each variant maps to a unique `i32` error code.
- Implement `From<...> for ErrorObjectOwned` so errors convert to JSON-RPC errors automatically.
- For errors that carry structured data, pass `Some(data)` instead of `None::<()>` in `ErrorObjectOwned::owned(...)`. Define the data struct with `Serialize + Deserialize`.
- Pick error codes that don't conflict with existing APIs. Check `crates/rpc/rpc-api/src/error/` for codes already in use.

### Register the error module

Add to `crates/rpc/rpc-api/src/error/mod.rs`:

```rust
pub mod <name>;
```

### Reference examples

- **Simple (code-as-discriminant)**: `error/katana.rs` — integer error codes via `#[repr(i32)]` enum discriminants.
- **With structured data**: `error/dev.rs` — `UnexpectedErrorData` passed as error data.
- **Feature-gated**: `error/tee.rs` — error codes starting at 100 to avoid conflicts.

---

## Step 4: Implement the Server Handler

Create a new module in `crates/rpc/rpc-server/src/<name>.rs` (or `crates/rpc/rpc-server/src/<name>/mod.rs` if the implementation is large enough to split into submodules).

### Structure

1. **Handler struct** — holds the state/dependencies the API needs.
2. **Internal methods** — business logic returning `Result<T, <Name>ApiError>`.
3. **Trait impl** — implements the `<Name>ApiServer` trait (generated by the proc macro), delegating to internal methods.

### Template

```rust
use std::sync::Arc;

use jsonrpsee::core::{async_trait, RpcResult};
use katana_rpc_api::<name>::<Name>ApiServer;
use katana_rpc_api::error::<name>::<Name>ApiError;

#[allow(missing_debug_implementations)]
pub struct <Name>Api {
    // Dependencies: storage providers, backend, etc.
    // Wrap shared state in Arc for cheap cloning.
}

impl <Name>Api {
    pub fn new(/* deps */) -> Self {
        Self { /* ... */ }
    }

    // Internal methods with concrete error types.
    fn some_internal_method(&self) -> Result<(), <Name>ApiError> {
        // ...
        Ok(())
    }
}

#[async_trait]
impl <Name>ApiServer for <Name>Api {
    async fn method_name(&self, param: ParamType) -> RpcResult<ResponseType> {
        // Delegate to internal method; the ? operator converts
        // <Name>ApiError -> ErrorObjectOwned automatically.
        Ok(self.some_internal_method()?)
    }
}
```

### Key patterns

- **Generic over storage**: If the handler needs storage access, make it generic over `ProviderFactory` (see `DevApi<PF>` and `TeeApi<PF>`).
- **Arc for shared state**: Wrap inner state in `Arc` if the handler needs to be cloned (required when registering with jsonrpsee).
- **Blocking tasks**: For I/O-heavy or CPU-heavy work, use `on_io_blocking_task` or `on_cpu_blocking_task` patterns (see `StarknetApi`). For simpler APIs this isn't needed.
- **Error conversion**: The `?` operator chains `From<ApiError> for ErrorObjectOwned` so that trait methods can use `Ok(self.internal_method()?)`.

### Register the module

Add to `crates/rpc/rpc-server/src/lib.rs`:

```rust
pub mod <name>;
```

If the API is feature-gated:

```rust
#[cfg(feature = "<feature>")]
pub mod <name>;
```

### Reference examples

- **Simple handler**: `crates/rpc/rpc-server/src/dev.rs` — `DevApi` with direct method calls.
- **Generic over storage**: `crates/rpc/rpc-server/src/tee.rs` — `TeeApi<PF>` parameterized by provider factory.
- **Complex handler with submodules**: `crates/rpc/rpc-server/src/starknet/` — split into `read.rs`, `write.rs`, `trace.rs`.

---

## Step 5: Register the API in the Sequencer Node (same for Full Node)

The final step is wiring the new API into the node so it actually gets served. Registration happens in `Node::build_with_provider` in `crates/node/sequencer/src/lib.rs`. There are also other node implementations (e.g., `crates/node/full/src/lib.rs`) that may need the same registration if applicable.

### 5a. Add a variant to `RpcModuleKind`

If the API should be toggleable at runtime (most APIs should be), add a variant to the `RpcModuleKind` enum in `crates/node/config/src/rpc.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RpcModuleKind {
    Starknet,
    Dev,
    Katana,
    // Add your new variant:
    <Name>,
}
```

For feature-gated APIs, annotate the variant:

```rust
#[cfg(feature = "<feature>")]
<Name>,
```

### 5b. Register in `Node::build_with_provider`

In `crates/node/sequencer/src/lib.rs`, add the imports and registration block. The registration goes in the `build_with_provider` method, in the section where other RPC modules are merged into `rpc_modules` (around lines 309–358).

**Add imports** at the top of the file:

```rust
use katana_rpc_api::<name>::<Name>ApiServer;
use katana_rpc_server::<name>::<Name>Api;
```

For feature-gated APIs, wrap the imports:

```rust
#[cfg(feature = "<feature>")]
use katana_rpc_api::<name>::<Name>ApiServer;
#[cfg(feature = "<feature>")]
use katana_rpc_server::<name>::<Name>Api;
```

**Add the registration block** alongside the existing API registrations:

```rust
// --- Always-on API (like Dev)
if config.rpc.apis.contains(&RpcModuleKind::<Name>) {
    let api = <Name>Api::new(/* deps from the build context: backend, pool, provider, etc. */);
    rpc_modules.merge(<Name>ApiServer::into_rpc(api))?;
}
```

For feature-gated APIs (like TEE):

```rust
#[cfg(feature = "<feature>")]
if config.rpc.apis.contains(&RpcModuleKind::<Name>) {
    let api = <Name>Api::new(/* deps */);
    rpc_modules.merge(<Name>ApiServer::into_rpc(api))?;
}
```

### Where to place it

The registration block goes **after** the existing API registrations and **before** the `RpcServer::new()` builder call. Follow the existing ordering in `build_with_provider`:

1. Paymaster/Cartridge APIs (feature-gated)
2. StarknetApi (read, write, trace)
3. KatanaApi
4. DevApi
5. TeeApi (feature-gated)
6. **Your new API goes here**
7. `RpcServer::new().module(rpc_modules)?` — builds the server

### Available dependencies

Inside `build_with_provider`, these objects are available to pass to your handler constructor:

| Variable | Type | Description |
|---|---|---|
| `backend` | `Arc<Backend<P>>` | Node backend (chain spec, executor, storage, gas oracle) |
| `block_producer` | `BlockProducer<P>` | Block production control |
| `pool` | `TxPool` | Transaction mempool |
| `provider` | `P` (impl `ProviderFactory`) | Storage provider factory |
| `task_spawner` | `TaskSpawner` | Async task spawner for blocking work |
| `gas_oracle` | `GasPriceOracle` | Gas price oracle |
| `config` | `Config` | Full node configuration |

### 5c. Don't forget other node implementations

If the API should also be available in the full node (not just the sequencer), apply the same registration in `crates/node/full/src/lib.rs`. The pattern is identical.

### 5d. Add `Cargo.toml` dependencies

Add the `katana-rpc-api` and `katana-rpc-server` crates as dependencies of the node crate (`crates/node/sequencer/Cargo.toml`) if they aren't already listed. For feature-gated APIs, gate the dependencies under the appropriate feature.

### Reference examples

Look at how existing APIs are registered in `crates/node/sequencer/src/lib.rs`:

- **Always-on, simple**: DevApi (lines 324–327) — conditional on `RpcModuleKind::Dev`.
- **Always-on, multi-trait**: StarknetApi (lines 309–322) — registers read, write, trace, and katana traits from the same handler.
- **Feature-gated**: TeeApi (lines 330–358) — guarded by `#[cfg(feature = "tee")]` and `RpcModuleKind::Tee`, with provider initialization logic.

---

## Checklist

- [ ] API trait defined in `crates/rpc/rpc-api/src/<name>.rs`
- [ ] Module added to `crates/rpc/rpc-api/src/lib.rs`
- [ ] Error enum defined in `crates/rpc/rpc-api/src/error/<name>.rs`
- [ ] Error module added to `crates/rpc/rpc-api/src/error/mod.rs`
- [ ] Custom types defined in `crates/rpc/rpc-types/src/` (if needed)
- [ ] Server handler implemented in `crates/rpc/rpc-server/src/<name>.rs`
- [ ] Handler module added to `crates/rpc/rpc-server/src/lib.rs`
- [ ] `RpcModuleKind` variant added in `crates/node/config/src/rpc.rs`
- [ ] API registered in `Node::build_with_provider` (`crates/node/sequencer/src/lib.rs`)
- [ ] API registered in full node if applicable (`crates/node/full/src/lib.rs`)
- [ ] Dependencies added to the relevant `Cargo.toml` files
