//! `saya-tee` sidecar process management.
//!
//! Spawns `saya-tee tee start --mock-prove ...` as a child process and
//! returns an RAII guard that kills the child on drop. The binary is
//! resolved via `SAYA_TEE_BIN` env var or `$PATH`.
//!
//! Build instructions:
//!
//! ```sh
//! cd dojoengine/saya  # rev: 5ff9948
//! cd bin/persistent-tee && cargo install --path .
//! ```
//!
//! In CI the build runs in a job that has the `cartridge-gg/katana-tee`
//! SSH deploy key loaded via `webfactory/ssh-agent`.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use anyhow::{anyhow, Context, Result};
use starknet_types_core::felt::Felt;
use tempfile::TempDir;
use url::Url;

#[derive(Debug, Clone)]
pub struct SayaTeeConfig {
    pub rollup_rpc: Url,
    pub settlement_rpc: Url,
    pub piltover_address: Felt,
    pub tee_registry_address: Felt,
    pub settlement_account_address: Felt,
    pub settlement_account_private_key: Felt,
}

/// RAII guard that kills the spawned `saya-tee` child process on drop.
pub struct SayaTeeGuard {
    child: Child,
    /// Temporary database directory; deleted on drop after the child is killed.
    _db_dir: TempDir,
}

impl Drop for SayaTeeGuard {
    fn drop(&mut self) {
        match self.child.try_wait() {
            Ok(Some(status)) => {
                eprintln!("[debug] saya-tee already exited before drop: {status:?}");
            }
            Ok(None) => {
                if let Err(e) = self.child.kill() {
                    eprintln!("failed to kill saya-tee child: {e}");
                }
                let _ = self.child.wait();
            }
            Err(e) => eprintln!("failed to query saya-tee child status: {e}"),
        }
    }
}

pub fn spawn_saya_tee(cfg: &SayaTeeConfig) -> Result<SayaTeeGuard> {
    let bin = resolve_saya_tee_bin()?;
    let db_dir = tempfile::tempdir().context("failed to create saya-tee db tempdir")?;

    let mut cmd = Command::new(&bin);
    cmd.args([
        "tee",
        "start",
        "--mock-prove",
        "--rollup-rpc",
        cfg.rollup_rpc.as_str(),
        "--settlement-rpc",
        cfg.settlement_rpc.as_str(),
        "--settlement-piltover-address",
        &hex(&cfg.piltover_address),
        "--tee-registry-address",
        &hex(&cfg.tee_registry_address),
        "--settlement-account-address",
        &hex(&cfg.settlement_account_address),
        "--settlement-account-private-key",
        &hex(&cfg.settlement_account_private_key),
        // `--prover-private-key` is required by clap but unused in mock-prove
        // mode (the SP1 prover network is never contacted). Pass a dummy.
        "--prover-private-key",
        "0xdeadbeef",
        "--db-dir",
        db_dir.path().to_str().context("db dir path is not valid utf8")?,
        // Flush every block immediately instead of accumulating up to the default
        // batch-size=10 (which, with our 3-block test, would always fall through
        // to the 120-second idle-timeout flush and dominate wall-clock time).
        "--batch-size",
        "1",
    ])
    .env("RUST_LOG", "info,persistent_tee=debug,saya_core=info")
    .stdout(Stdio::inherit())
    .stderr(Stdio::inherit());

    println!("spawning saya-tee from {bin:?}");
    let child = cmd.spawn().with_context(|| format!("failed to spawn saya-tee at {bin:?}"))?;

    Ok(SayaTeeGuard { child, _db_dir: db_dir })
}

fn resolve_saya_tee_bin() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("SAYA_TEE_BIN") {
        return Ok(PathBuf::from(path));
    }
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join("saya-tee");
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(anyhow!(
        "`saya-tee` binary not found. Set SAYA_TEE_BIN env var or add it to $PATH. Build from \
         dojoengine/saya@5a3b8c9 with `cd bin/persistent-tee && cargo install --path .`"
    ))
}

fn hex(felt: &Felt) -> String {
    format!("0x{:x}", felt)
}
