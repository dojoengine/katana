//! VRF sidecar process management.
//!
//! This module handles spawning and managing the VRF sidecar process.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use tokio::process::{Child, Command};
use tokio::time::sleep;
use tracing::{debug, info, warn};

use super::bootstrap::VrfBootstrapResult;

const LOG_TARGET: &str = "katana::cartridge::vrf::sidecar";

/// Fixed port used by vrf-server.
pub const VRF_SERVER_PORT: u16 = 3000;

/// Default timeout for waiting on sidecar readiness.
pub const SIDECAR_TIMEOUT: Duration = Duration::from_secs(10);

/// Sidecar-specific info for VRF (used by CLI to start sidecar process).
#[derive(Debug, Clone)]
pub struct VrfSidecarInfo {
    pub port: u16,
}

/// Configuration for the VRF sidecar.
#[derive(Debug, Clone)]
pub struct VrfSidecarConfig {
    /// Optional path to the VRF sidecar binary.
    pub bin: Option<PathBuf>,
    /// Port to bind the sidecar on.
    pub port: u16,
}

/// Start the VRF sidecar process.
pub async fn start_vrf_sidecar(
    config: &VrfSidecarConfig,
    bootstrap: &VrfBootstrapResult,
) -> Result<Child> {
    if config.port != VRF_SERVER_PORT {
        return Err(anyhow!(
            "vrf-server uses a fixed port of {VRF_SERVER_PORT}; set --vrf.port={VRF_SERVER_PORT}"
        ));
    }

    let bin = config.bin.clone().unwrap_or_else(|| "vrf-server".into());
    let bin = resolve_executable(Path::new(&bin))?;

    let mut command = Command::new(bin);
    command
        .arg("--secret-key")
        .arg(bootstrap.secret_key.to_string())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);

    let child = command.spawn().context("failed to spawn vrf sidecar")?;

    let url = format!("http://127.0.0.1:{}/info", config.port);
    wait_for_http_ok(&url, "vrf info", SIDECAR_TIMEOUT).await?;

    Ok(child)
}

/// Resolve an executable path, searching in PATH if necessary.
pub fn resolve_executable(path: &Path) -> Result<PathBuf> {
    if path.components().count() > 1 {
        return if path.is_file() {
            Ok(path.to_path_buf())
        } else {
            Err(anyhow!("sidecar binary not found at {}", path.display()))
        };
    }

    let path_var = env::var_os("PATH").ok_or_else(|| anyhow!("PATH is not set"))?;
    for dir in env::split_paths(&path_var) {
        let candidate = dir.join(path);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    Err(anyhow!("sidecar binary '{}' not found in PATH", path.display()))
}

/// Wait for an HTTP endpoint to return a successful response.
pub async fn wait_for_http_ok(url: &str, name: &str, timeout: Duration) -> Result<()> {
    let client = reqwest::Client::new();
    let start = Instant::now();

    loop {
        match client.get(url).send().await {
            Ok(resp) if resp.status().is_success() => {
                info!(target: LOG_TARGET, %name, "sidecar ready");
                return Ok(());
            }
            Ok(resp) => {
                debug!(target: LOG_TARGET, %name, status = %resp.status(), "waiting for sidecar");
            }
            Err(err) => {
                debug!(target: LOG_TARGET, %name, error = %err, "waiting for sidecar");
            }
        }

        if start.elapsed() > timeout {
            warn!(target: LOG_TARGET, %name, "sidecar did not become ready in time");
            return Err(anyhow!("{} did not become ready before timeout", name));
        }

        sleep(Duration::from_millis(200)).await;
    }
}
