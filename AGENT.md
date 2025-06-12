# Katana Development Guide for AI Coding Tools

## Build/Test/Lint Commands
- Build: `cargo build --all-features`
- Test all: `cargo nextest run --all-features --workspace`
- Test single: `cargo nextest run -p <crate-name> --test <test-name>`
- Unit test: `cargo test <test-function-name>`
- Lint: `./scripts/clippy.sh` (uses nightly toolchain)
- Format: `./scripts/rust_fmt.sh` or `cargo +nightly fmt`
- Check: `cargo check --all-features --workspace`

## Code Style & Conventions
- Rust edition 2021, toolchain 1.85.0
- Line width: 100 chars (rustfmt.toml)
- Import grouping: StdExternalCrate, module granularity
- Use field init shorthand, try shorthand
- Error handling: `anyhow::Result` for functions, `thiserror::Error` for custom errors
- Testing: `#[tokio::test]` for async, `rstest` for parameterized, `assert_matches` for patterns
- Naming: snake_case functions, descriptive test names like `should_fail_when_invalid`
- Workspace structure: each crate in `crates/` with `katana-` prefix

## Testing Structure
- Unit tests: inline `#[cfg(test)]` modules
- Integration tests: separate files in `tests/` dirs
- Use `anyhow::Result` in tests, `tokio::test` for async
- Test artifacts: `make test-artifacts` to prepare fixtures
