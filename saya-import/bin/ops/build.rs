//! Rebuilds Piltover mock contracts from source via the `piltover`
//! submodule + scarb. The submodule is the single source of truth for the
//! Sierra bytecode embedded in saya-ops.
//!
//! Dev setup: run `make install-scarb` from the repo root before `cargo build`.
//! This build script hard-fails if asdf/scarb is missing — there is no
//! vendored fallback for the mock contracts.
//!
//! NOT rebuilt: `contracts/core_contract.json`. Its class hash is load-bearing
//! for already-deployed Piltover on Sepolia/Mainnet, and the Cairo source in
//! the pinned piltover rev has diverged from the rev it was originally
//! compiled at. It remains vendored.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Pairs of (piltover scarb output name, OUT_DIR name for include_bytes!).
const CONTRACTS: &[(&str, &str)] = &[
    ("piltover_fact_registry_mock.contract_class.json", "fact_registry_mock.json"),
    ("piltover_mock_amd_tee_registry.contract_class.json", "tee_registry_mock.json"),
];

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let workspace_root = manifest_dir.join("../..");
    let piltover_dir = workspace_root.join("piltover");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../piltover/src");
    println!("cargo:rerun-if-changed=../../piltover/Scarb.toml");
    println!("cargo:rerun-if-changed=../../piltover/Scarb.lock");
    println!("cargo:rerun-if-changed=../../piltover/.tool-versions");

    // Auto-init the submodule if it's empty (user may have cloned without
    // `--recursive`). Mirrors katana's `initialize_submodule` helper in
    // `crates/contracts/build.rs`.
    initialize_submodule_if_empty(&piltover_dir);

    // Hard-fail if scarb isn't on PATH. Dev must run `make install-scarb` first
    // (or install scarb directly). We invoke `scarb` directly rather than
    // `asdf exec scarb` so CI environments that install scarb via the official
    // installer (without asdf) also work. Local dev via asdf still works
    // transparently: asdf shims intercept the `scarb` call and route to the
    // version pinned by `piltover/.tool-versions`.
    let scarb_check = Command::new("scarb")
        .arg("--version")
        .current_dir(&piltover_dir)
        .output();
    if !scarb_check.as_ref().map(|o| o.status.success()).unwrap_or(false) {
        panic!(
            "scarb not found on PATH. Run `make install-scarb` from the repo root to \
             install the version pinned by `piltover/.tool-versions`, or install scarb \
             directly from https://docs.swmansion.com/scarb/download.html."
        );
    }

    // Build piltover contracts via scarb.
    let status = Command::new("scarb")
        .arg("build")
        .current_dir(&piltover_dir)
        .status()
        .expect("`scarb build` failed to spawn");
    if !status.success() {
        panic!("scarb build failed in {}", piltover_dir.display());
    }

    // Copy the two mock artifacts into OUT_DIR under stable names so the
    // corresponding `include_bytes!` sites in `core_contract/constants.rs`
    // can resolve them.
    for (scarb_name, out_name) in CONTRACTS {
        let src = piltover_dir.join("target/dev").join(scarb_name);
        let dst = out_dir.join(out_name);
        std::fs::copy(&src, &dst).unwrap_or_else(|e| {
            panic!(
                "failed to copy scarb output {} -> {}: {e}",
                src.display(),
                dst.display()
            )
        });
    }
}

/// Auto-initializes the piltover submodule if the working tree is empty.
/// Prints a `cargo:warning` so the user knows it's happening.
fn initialize_submodule_if_empty(submodule_dir: &Path) {
    if submodule_dir.join("Scarb.toml").exists() {
        return;
    }
    println!(
        "cargo:warning=piltover submodule at {} not initialized, running `git submodule \
         update --init --recursive --force`...",
        submodule_dir.display()
    );
    let in_git_repo = Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !in_git_repo {
        panic!(
            "Not in a git repo, and {} is empty. Clone saya with `--recursive`, or run \
             `git submodule update --init --recursive` from a proper checkout.",
            submodule_dir.display()
        );
    }
    let status = Command::new("git")
        .args(["submodule", "update", "--init", "--recursive", "--force"])
        .arg(submodule_dir)
        .status()
        .expect("`git submodule update` failed to spawn");
    if !status.success() {
        panic!("Failed to initialize piltover submodule at {}", submodule_dir.display());
    }
}
