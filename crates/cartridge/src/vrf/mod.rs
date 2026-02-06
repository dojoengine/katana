//! VRF (Verifiable Random Function) support for Cartridge.
//!
//! This module provides:
//! - VRF client for communicating with the VRF server
//! - Bootstrap logic for deploying VRF contracts
//! - Sidecar process management

mod bootstrap;
mod client;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};
use std::{env, io};

pub use bootstrap::{
    bootstrap_vrf, get_vrf_account, VrfAccountCredentials, VrfBootstrap, VrfBootstrapConfig,
    VrfBootstrapResult, BOOTSTRAP_TIMEOUT, VRF_ACCOUNT_SALT, VRF_CONSUMER_SALT,
    VRF_HARDCODED_SECRET_KEY,
};
pub use client::*;
use katana_primitives::{ContractAddress, Felt};
use tokio::process::{Child, Command};
use tokio::time::sleep;
use tracing::{debug, info, warn};
use url::Url;

const LOG_TARGET: &str = "katana::cartridge::vrf::sidecar";

pub const VRF_SERVER_PORT: u16 = 3000;
const DEFAULT_VRF_SERVICE_PATH: &str = "vrf-server";
pub const SIDECAR_TIMEOUT: Duration = Duration::from_secs(10);

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

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Clone)]
pub struct VrfServiceConfig {
    pub vrf_account_address: ContractAddress,
    pub vrf_private_key: Felt,
    pub secret_key: u64,
}

#[derive(Debug, Clone)]
pub struct VrfService {
    config: VrfServiceConfig,
    path: PathBuf,
}

impl VrfService {
    pub fn new(config: VrfServiceConfig) -> Self {
        Self { config, path: PathBuf::from(DEFAULT_VRF_SERVICE_PATH) }
    }

    /// Sets the path to the vrf service program.
    ///
    /// If no path is set, the default executable name [`DEFAULT_VRF_SERVICE_PATH`] will be used.
    pub fn path<T: Into<PathBuf>>(mut self, path: T) -> Self {
        self.path = path.into();
        self
    }

    pub async fn start(self) -> Result<VrfServiceProcess> {
        let bin = resolve_executable(&self.path)?;

        let mut command = Command::new(bin);
        command
            .arg("--port")
            .arg(VRF_SERVER_PORT.to_string())
            .arg("--account-address")
            .arg(self.config.vrf_account_address.to_hex_string())
            .arg("--account-private-key")
            .arg(self.config.vrf_private_key.to_hex_string())
            .arg("--secret-key")
            .arg(self.config.secret_key.to_string())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);

        let process = command.spawn().map_err(Error::Spawn)?;
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), VRF_SERVER_PORT);

        let url = Url::parse(&format!("http://{addr}")).expect("valid url");
        let client = VrfClient::new(url);
        wait_for_http_ok(&client, "vrf info", SIDECAR_TIMEOUT).await?;

        info!(%addr, "VRF service started.");

        Ok(VrfServiceProcess { process, addr, inner: self })
    }
}

/// A running VRF sidecar process.
#[derive(Debug)]
pub struct VrfServiceProcess {
    process: Child,
    inner: VrfService,
    addr: SocketAddr,
}

impl VrfServiceProcess {
    /// Get the address of the VRF service.
    pub fn addr(&self) -> &SocketAddr {
        &self.addr
    }

    pub fn process(&mut self) -> &mut Child {
        &mut self.process
    }

    pub fn config(&self) -> &VrfServiceConfig {
        &self.inner.config
    }

    pub async fn shutdown(&mut self) -> io::Result<()> {
        self.process.kill().await
    }
}

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

/// Wait for the VRF sidecar to become ready by polling its `/info` endpoint.
pub async fn wait_for_http_ok(client: &VrfClient, name: &str, timeout: Duration) -> Result<()> {
    let start = Instant::now();

    loop {
        match client.info().await {
            Ok(_) => {
                info!(target: LOG_TARGET, %name, "sidecar ready");
                return Ok(());
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
