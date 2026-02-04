//! VRF sidecar process management.
//!
//! This module handles spawning and managing the VRF sidecar process.

use std::env;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use katana_primitives::{ContractAddress, Felt};
use tokio::process::{Child, Command};
use tokio::time::sleep;
use tracing::{debug, info, warn};
use url::Url;

use super::bootstrap::{bootstrap_vrf, VrfBootstrapConfig, VrfBootstrapResult};

const LOG_TARGET: &str = "katana::cartridge::vrf::sidecar";

/// Fixed port used by vrf-server.
pub const VRF_SERVER_PORT: u16 = 3000;

/// Default timeout for waiting on sidecar readiness.
pub const SIDECAR_TIMEOUT: Duration = Duration::from_secs(10);

// ============================================================================
// Error Types
// ============================================================================

/// Error type for VRF sidecar operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("bootstrap_result not set - call bootstrap() or bootstrap_result()")]
    BootstrapResultNotSet,
    #[error("sidecar binary not found at {0}")]
    BinaryNotFound(PathBuf),
    #[error("sidecar binary '{0}' not found in PATH")]
    BinaryNotInPath(PathBuf),
    #[error("PATH environment variable is not set")]
    PathNotSet,
    #[error("failed to spawn VRF sidecar")]
    Spawn(#[source] io::Error),
    #[error("VRF sidecar did not become ready before timeout")]
    SidecarTimeout,
    #[error("bootstrap failed")]
    Bootstrap(#[source] anyhow::Error),
}

/// Result type alias for VRF sidecar operations.
pub type Result<T, E = Error> = std::result::Result<T, E>;

// ============================================================================
// Configuration Types
// ============================================================================

/// Configuration for the VRF service.
#[derive(Debug, Clone)]
pub struct VrfServiceConfig {
    /// RPC URL of the katana node (for bootstrap).
    pub rpc_url: Url,
    /// Source account address (deploys contracts and funds VRF account).
    pub source_address: ContractAddress,
    /// Source account private key.
    pub source_private_key: Felt,
    /// Path to the vrf-server binary (None = lookup in PATH).
    pub program_path: Option<PathBuf>,
}

// ============================================================================
// VRF Service
// ============================================================================

/// VRF service that handles bootstrapping and spawning the sidecar process.
#[derive(Debug, Clone)]
pub struct VrfService {
    config: VrfServiceConfig,
    bootstrap_result: Option<VrfBootstrapResult>,
}

impl VrfService {
    /// Create a new VRF service with the given configuration.
    pub fn new(config: VrfServiceConfig) -> Self {
        Self { config, bootstrap_result: None }
    }

    /// Set a pre-existing bootstrap result, skipping the bootstrap step.
    ///
    /// Use this when the VRF contracts have already been deployed.
    pub fn bootstrap_result(mut self, result: VrfBootstrapResult) -> Self {
        self.bootstrap_result = Some(result);
        self
    }

    /// Get the bootstrap result, if set.
    pub fn get_bootstrap_result(&self) -> Option<&VrfBootstrapResult> {
        self.bootstrap_result.as_ref()
    }

    /// Bootstrap the VRF service by deploying necessary contracts.
    ///
    /// This deploys the VRF account and consumer contracts via RPC,
    /// sets up the VRF public key, and optionally funds the account.
    pub async fn bootstrap(&mut self) -> Result<&VrfBootstrapResult> {
        let bootstrap_config = VrfBootstrapConfig {
            rpc_url: self.config.rpc_url.clone(),
            source_address: self.config.source_address,
            source_private_key: self.config.source_private_key,
        };

        let result = bootstrap_vrf(&bootstrap_config).await.map_err(Error::Bootstrap)?;
        self.bootstrap_result = Some(result);

        Ok(self.bootstrap_result.as_ref().expect("just set"))
    }

    /// Start the VRF sidecar process.
    ///
    /// This spawns the vrf-server binary and waits for it to become ready.
    /// Returns an error if bootstrap has not been performed.
    pub async fn start(self) -> Result<VrfServiceProcess> {
        let bootstrap_result = self.bootstrap_result.ok_or(Error::BootstrapResultNotSet)?.clone();

        let bin = self.config.program_path.unwrap_or_else(|| "vrf-server".into());
        let bin = resolve_executable(Path::new(&bin))?;

        let mut command = Command::new(bin);
        command
            .arg("--port")
            .arg(VRF_SERVER_PORT.to_string())
            .arg("--account-address")
            .arg("--account-private-key")
            .arg("--secret-key")
            .arg(bootstrap_result.secret_key.to_string())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);

        let process = command.spawn().map_err(Error::Spawn)?;

        let url = format!("http://127.0.0.1:{VRF_SERVER_PORT}/info",);
        wait_for_http_ok(&url, "vrf info", SIDECAR_TIMEOUT).await?;

        Ok(VrfServiceProcess { process, bootstrap_result })
    }
}

// ============================================================================
// VRF Sidecar Process
// ============================================================================

/// A running VRF sidecar process.
#[derive(Debug)]
pub struct VrfServiceProcess {
    process: Child,
    bootstrap_result: VrfBootstrapResult,
}

impl VrfServiceProcess {
    /// Get a mutable reference to the underlying child process.
    pub fn process(&mut self) -> &mut Child {
        &mut self.process
    }

    /// Get the bootstrap result containing VRF account information.
    pub fn bootstrap_result(&self) -> &VrfBootstrapResult {
        &self.bootstrap_result
    }

    /// Gracefully shutdown the sidecar process.
    pub async fn shutdown(&mut self) -> io::Result<()> {
        self.process.kill().await
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Resolve an executable path, searching in PATH if necessary.
pub fn resolve_executable(path: &Path) -> Result<PathBuf> {
    if path.components().count() > 1 {
        return if path.is_file() {
            Ok(path.to_path_buf())
        } else {
            Err(Error::BinaryNotFound(path.to_path_buf()))
        };
    }

    let path_var = env::var_os("PATH").ok_or(Error::PathNotSet)?;
    for dir in env::split_paths(&path_var) {
        let candidate = dir.join(path);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    Err(Error::BinaryNotInPath(path.to_path_buf()))
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
            return Err(Error::SidecarTimeout);
        }

        sleep(Duration::from_millis(200)).await;
    }
}
