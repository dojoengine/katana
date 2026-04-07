//! `katana-bootstrap` — declare classes and deploy contracts on a running Katana node.
//!
//! This crate is the home of everything behind the `katana bootstrap` subcommand:
//!
//! - [`embedded`]   — registry of Sierra classes baked into the binary at compile time
//! - [`manifest`]   — TOML schema users feed into programmatic mode
//! - [`plan`]       — resolved, ready-to-execute representation of a manifest
//! - [`executor`]   — RPC executor that submits declares/deploys via `starknet-rs`
//! - [`tui`]        — full ratatui-based interactive UI
//! - [`report`]     — terminal pretty-printer for the post-execution summary
//!
//! [`BootstrapArgs`] is the [`clap`] entry point that `bin/katana` mounts as the
//! `bootstrap` subcommand. It's the only thing the binary needs to know about — every
//! other module here is reachable through it but can also be used standalone (e.g. by
//! tests or downstream tooling).

pub mod embedded;
pub mod executor;
pub mod manifest;
pub mod plan;
pub mod report;
pub mod tui;

mod cli;

pub use cli::BootstrapArgs;
