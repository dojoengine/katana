---
name: katana-development
description: Contributor workflow for dojoengine/katana. Use when implementing or reviewing changes in Katana's Rust crates, native/explorer integration, fixtures, and CI-parity checks.
---

# Katana Development

Use this skill to contribute safely to `dojoengine/katana` with the expected dependency setup and test flow.

## Core Workflow

1. Prepare dependencies before builds/tests:
   - `make install-scarb`
   - `make native-deps-macos` or `make native-deps-linux`
2. Build targets based on scope:
   - `cargo build`
   - `cargo build --release`
   - `make build-explorer` when explorer assets are affected
3. Refresh fixtures before test runs:
   - `make fixtures`
4. Run tests:
   - `cargo nextest run`
   - Use `cargo nextest run -p <crate_name>` for focused iteration
5. Run formatting and lint checks:
   - `cargo +nightly-2025-02-20 fmt --all`
   - `./scripts/clippy.sh`

## PR Checklist

- Run `make fixtures` before final test pass.
- Note whether explorer/native paths were touched.
- Include exact commands used for validation.
