# Repository Guidelines

## Project Structure & Module Organization
- `src/` holds the Cairo contracts and tests (`src/tests/`), plus modules like `vrf_provider/`, `vrf_account/`, and shared types in `types.cairo`.
- `server/` is the Rust VRF HTTP server. App code lives in `server/src/`, and Rust tests live under `server/src/tests/`.
- `dojo/` and `js/` contain supporting scripts/utilities (TypeScript/JavaScript examples and helpers).
- `accounts/` contains local/dev key material for testing. Treat as non-production and avoid committing real secrets.

## Build, Test, and Development Commands
- Cairo contracts (from repo root):
  - `scarb fmt --check` — format check for Cairo sources.
  - `scarb build` — compile Cairo contracts.
  - `snforge test` — run Cairo tests (configured in `Scarb.toml`).
- Rust server:
  - `cd server && cargo build --release` — build the server binary.
  - `cd server && cargo run -- --host 0.0.0.0 --port 3000 --secret-key <u64> --account-address <hex> --account-private-key <hex>` — run locally.
  - `cd server && cargo test --workspace --verbose` — run Rust tests.
- Optional linting:
  - `cd server && cargo fmt --all` and `cd server && cargo clippy` — formatting and linting used in CI.

## Coding Style & Naming Conventions
- Rust: follow `rustfmt`/`clippy`; use 4-space indentation; `snake_case` for functions/modules, `PascalCase` for types, `SCREAMING_SNAKE_CASE` for constants.
- Cairo: follow `scarb fmt`; use conventional Cairo naming (similar to Rust).
- JS scripts in `js/` use ESM (`"type": "module"`) and 2-space indentation; keep them small and task-focused.

## Testing Guidelines
- Cairo tests live in `src/tests/` and run via `snforge test`.
- Rust tests live in `server/src/tests/` and run via `cargo test`.
- Prefer deterministic tests; when touching VRF logic, add both Cairo and Rust coverage where applicable.

## Commit & Pull Request Guidelines
- Commit messages are short and imperative; many use conventional prefixes like `feat:`, `fix:`, or `chore:`. Prefer those when applicable.
- PRs should include a clear summary, test evidence (commands + results), and link relevant issues.
- If you change CLI behavior or APIs, update `README.md`/`server/README.md` and include example usage.

## Security & Configuration Tips
- Never commit real private keys. Use placeholders in docs and tests.
- Tagging `vX.Y.Z` triggers release workflows; ensure CI is green before tagging.
