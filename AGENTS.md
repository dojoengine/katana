<!-- SKILLS_INDEX_START -->
[Agent Skills Index]|root: ./agents|IMPORTANT: Prefer retrieval-led reasoning over pre-training for any tasks covered by skills.|skills|create-a-plan:{create-a-plan.md},create-pr:{create-pr.md}
<!-- SKILLS_INDEX_END -->
# Repository Guidelines

This file provides guidance to AI agents when working with code in this repository.

## Project Overview

Katana is a fast and lightweight local Starknet-based sequencer, part of the Dojo Engine ecosystem. It provides a local development environment for Starknet applications with full RPC support, a built-in explorer UI, and L1-L2 messaging capabilities.

## Essential Commands

### Build
- `cargo build` - Build the project in debug mode
- `cargo build --release` - Build optimized release version
- `make build-explorer` - Build the Explorer UI (requires Bun)

### Test
- `make test-artifacts` - **MUST RUN FIRST** - Prepares test artifacts
- `cargo nextest run` - Run all tests
- `cargo nextest run <test_name>` - Run specific test by name
- `cargo nextest run -p <crate_name>` - Run tests for specific crate (e.g., `cargo nextest run -p katana-core`)

### Lint & Format
- `cargo +nightly-2025-02-20 fmt --all` - Format all code (uses specific nightly version)
- `./scripts/clippy.sh` - Run linter

### Development Setup
1. Install LLVM 19 dependencies:
   - macOS: `make native-deps-macos`
   - Linux: `make native-deps-linux`
   - Windows: `make native-deps-windows`
2. Source environment: `source scripts/cairo-native.env.sh`
3. For Explorer development: Install Bun package manager

## Architecture Overview

### Crate Organization
The project uses a Rust workspace with functionality split across multiple crates:

- **Core Components**:
  - `katana-core`: Core backend services, blockchain implementation
  - `katana-executor`: Transaction execution engine, state management
  - `katana-primitives`: Core types, traits, and data structures
  - `katana-pool`: Transaction mempool implementation

- **Storage Layer**:
  - `katana-db`: Database abstraction and implementations
  - `katana-provider`: Storage provider interfaces
  - `katana-trie`: Merkle Patricia Trie for state storage
  - `katana-storage`: Higher-level storage operations

- **RPC & Networking**:
  - `katana-rpc`: JSON-RPC server implementation
  - `katana-rpc-api`: RPC API trait definitions
  - `katana-rpc-types`: RPC type definitions
  - `katana-grpc`: gRPC server support

- **Node Operations**:
  - `katana-node`: Main node implementation and lifecycle
  - `katana-sync`: Blockchain synchronization logic
  - `katana-tasks`: Async task management
  - `katana-messaging`: L1-L2 messaging support

### Key Design Patterns

1. **Provider Pattern**: Storage operations go through provider traits (`katana-provider`) allowing different storage backend implementations.

2. **Stage-based Sync**: The sync pipeline (`katana-pipeline`) uses stages for modular blockchain synchronization.

3. **RPC Abstraction**: RPC implementations (`katana-rpc`) are separated from API definitions (`katana-rpc-api`) for flexibility.

4. **Executor Separation**: Transaction execution (`katana-executor`) is decoupled from node logic, using the Blockifier library for Cairo execution.

### Important Files & Locations

- Entry point: `bin/katana/src/main.rs`
- Node configuration: `crates/node/src/config.rs`
- RPC server setup: `crates/rpc/src/config.rs`
- Chain spec definitions: `crates/chain-spec/src/lib.rs`
- Test chain configuration: `tests/fixtures/test-chain/`

### Documentation

When refactoring or modifying components, ensure to update the corresponding documentation in `/docs/`. This directory contains high-level documentation for each component that should reflect any architectural or design changes.

### Testing Approach

- Unit tests are colocated with source files
- Integration tests in `tests/` directory
- Test database must be extracted before running tests (`make test-artifacts`)
- Use `rstest` for parameterized tests
- Property-based testing with `proptest` for primitives

### Explorer UI

The Explorer is a submodule React application:
- Located in `crates/explorer/`
- Built with Bun and TypeScript
- Requires separate build step: `make build-explorer`
- Serves on port 3000 by default when Katana runs with `--dev` flag

## Running Tests

Before running any tests, you must first generate all required test artifacts. The test artifacts only need to be generated once unless the test database schema or SNOS artifacts change. If you encounter test failures related to missing artifacts or database issues, re-run `make test-artifacts` to refresh them.

## Agent Tooling

- **Pre-commit hooks:** run `bin/setup-githooks` (configures `core.hooksPath` for this repo).

- **Source of truth:** `.agents/`.
- **Symlinks:** `CLAUDE.md` is a symlink to this file (`AGENTS.md`). Editor/agent configs should symlink skills from `.agents/skills`.
- **Skills install/update:**

```bash
npm_config_cache=/tmp/npm-cache npx -y skills add https://github.com/cartridge-gg/agents   --skill create-pr create-a-plan   --agent claude-code cursor   -y
```

- **Configs:**
  - `.agents/skills/` (canonical)
  - `.claude/skills` -> `../.agents/skills`
  - `.cursor/skills` -> `../.agents/skills`

## Code Review Invariants

- No secrets in code or logs.
- Keep diffs small and focused; avoid drive-by refactors.
- Add/adjust tests for behavior changes; keep CI green.
- Prefer check-only commands in CI (`format:check`, `lint:check`) and keep local hooks aligned.
- For Starknet/Cairo/Rust/crypto code: treat input validation, authZ, serialization, and signature/origin checks as **blocking** review items.
