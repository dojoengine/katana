# Katana Development Guide

## Build Commands
- `cargo build` - Build the project
- `cargo test` - Run all tests  
- `cargo test <test_name>` - Run a specific test
- `cargo test -p <crate_name>` - Run tests for a specific crate
- `cargo clippy` - Run linter
- `make test-artifacts` - Prepare test database and SNOS artifacts

## Code Style
- **Formatting**: Uses rustfmt with 100 char width, group imports by StdExternalCrate
- **Imports**: std → external crates → local crates, use explicit paths `use crate::module::Type`
- **Naming**: snake_case functions, constructors use `new()`, `from_*()`, `with_*()` patterns
- **Error Handling**: Use `thiserror` for custom errors, `anyhow` for general error handling, always use `Result<T>` types
- **Visibility**: Clear `pub` boundaries, comprehensive `///` docs for public APIs
- **Async**: Use `async/await` with `?` operator for error propagation
- **Types**: Use `Arc<>` for shared ownership, prefer owned types over references in function signatures

## Testing
- Test modules use `#[cfg(test)]` 
- Use descriptive test names that explain the scenario being tested
- Use `rstest` for parameterized tests, `assert_matches` for pattern matching
- Run `make test-artifacts` before running tests that need database fixtures

## Architecture
- Workspace with multiple crates under `crates/` directory
- Each crate focuses on specific functionality (rpc, storage, executor, etc.)
- Use workspace dependencies for consistent versioning across crates
