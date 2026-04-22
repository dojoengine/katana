//! Emits build-time identity constants (git SHA, build timestamp, dev-build suffix)
//! that power [`BuildInfo::current`](crate::build_info::BuildInfo::current).
//!
//! If git metadata is unavailable (e.g. this crate is consumed from crates.io without
//! a `.git` directory), vergen fails silently and the `option_env!` fallbacks in
//! `build_info.rs` produce `"unknown"` sentinels instead of breaking the build.

use std::env;
use std::error::Error;

use vergen::{BuildBuilder, Emitter};
use vergen_gitcl::GitclBuilder;

fn main() -> Result<(), Box<dyn Error>> {
    // Best-effort: if vergen can't read git metadata (no .git, shallow clone, etc.),
    // we fall through and let option_env!() in src code produce "unknown" sentinels.
    if let Err(err) = emit_vergen() {
        println!(
            "cargo:warning=katana-node-config: vergen unavailable ({err}); build_info will use \
             \"unknown\" sentinels"
        );
    }
    Ok(())
}

fn emit_vergen() -> Result<(), Box<dyn Error>> {
    let build = BuildBuilder::default().build_timestamp(true).build()?;
    let gitcl =
        GitclBuilder::default().describe(true, false, None).dirty(true).sha(true).build()?;

    Emitter::default().add_instructions(&build)?.add_instructions(&gitcl)?.emit_and_set()?;

    // DEV_BUILD_SUFFIX is "-dev" when the working tree is dirty or the current revision
    // is not on a tag; empty otherwise. Downstream uses it to render version strings
    // like "1.0.0-alpha.19-dev".
    let sha = env::var("VERGEN_GIT_SHA")?;
    let is_dirty = env::var("VERGEN_GIT_DIRTY")? == "true";
    let not_on_tag = env::var("VERGEN_GIT_DESCRIBE")?.ends_with(&format!("-g{sha}"));
    let is_dev = is_dirty || not_on_tag;
    println!("cargo:rustc-env=DEV_BUILD_SUFFIX={}", if is_dev { "-dev" } else { "" });

    Ok(())
}
