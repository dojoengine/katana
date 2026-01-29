//! Sidecar process management for CLI.
//!
//! This module handles spawning and managing sidecar processes (paymaster, VRF)
//! when running in sidecar mode. The node treats all services as external - this
//! module bridges the gap by spawning and managing the sidecar processes.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::{env, fs};

use anyhow::{anyhow, Context, Result};
use katana_node::sidecar::{format_felt, BootstrapResult, PaymasterBootstrap};
use katana_primitives::chain::{ChainId, NamedChainId};
use katana_primitives::Felt;
use serde::Serialize;
use tokio::process::{Child, Command};
use tokio::time::sleep;
use tracing::{debug, info, warn};
use url::Url;

use crate::options::PaymasterOptions;
#[cfg(feature = "vrf")]
use crate::options::VrfOptions;

const BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(feature = "vrf")]
const VRF_SERVER_PORT: u16 = 3000;
const DEFAULT_AVNU_PRICE_SEPOLIA_ENDPOINT: &str = "https://sepolia.api.avnu.fi";
const DEFAULT_AVNU_PRICE_MAINNET_ENDPOINT: &str = "https://starknet.api.avnu.fi";

/// Manages sidecar child processes.
///
/// When dropped, the sidecar processes are killed.
#[derive(Debug)]
pub struct SidecarProcesses {
    paymaster: Option<Child>,
    #[cfg(feature = "vrf")]
    vrf: Option<Child>,
}

impl SidecarProcesses {
    #[cfg(feature = "vrf")]
    pub fn new(paymaster: Option<Child>, vrf: Option<Child>) -> Self {
        Self { paymaster, vrf }
    }

    #[cfg(not(feature = "vrf"))]
    pub fn new(paymaster: Option<Child>) -> Self {
        Self { paymaster }
    }

    /// Gracefully shutdown all sidecar processes.
    ///
    /// This kills each process and waits for it to exit.
    pub async fn shutdown(&mut self) {
        if let Some(ref mut child) = self.paymaster {
            info!(target: "sidecar", "shutting down paymaster sidecar");
            let _ = child.kill().await;
        }
        #[cfg(feature = "vrf")]
        if let Some(ref mut child) = self.vrf {
            info!(target: "sidecar", "shutting down vrf sidecar");
            let _ = child.kill().await;
        }
    }
}

impl Drop for SidecarProcesses {
    fn drop(&mut self) {
        if let Some(mut child) = self.paymaster.take() {
            let _ = child.start_kill();
        }
        #[cfg(feature = "vrf")]
        if let Some(mut child) = self.vrf.take() {
            let _ = child.start_kill();
        }
    }
}

/// Configuration for starting sidecars.
pub struct SidecarStartConfig<'a> {
    pub paymaster: Option<PaymasterSidecarConfig<'a>>,
    #[cfg(feature = "vrf")]
    pub vrf: Option<VrfSidecarConfig<'a>>,
}

/// Configuration for the paymaster sidecar.
pub struct PaymasterSidecarConfig<'a> {
    pub options: &'a PaymasterOptions,
    pub port: u16,
    pub api_key: String,
}

/// Configuration for the VRF sidecar.
#[cfg(feature = "vrf")]
pub struct VrfSidecarConfig<'a> {
    pub options: &'a VrfOptions,
    pub port: u16,
}

/// Start sidecar processes using the bootstrap data from the node.
pub async fn start_sidecars(
    config: &SidecarStartConfig<'_>,
    bootstrap: &BootstrapResult,
    rpc_addr: &SocketAddr,
) -> Result<SidecarProcesses> {
    let mut paymaster_child = None;
    #[cfg(feature = "vrf")]
    let mut vrf_child = None;

    if let (Some(paymaster_cfg), Some(paymaster_bootstrap)) =
        (&config.paymaster, bootstrap.paymaster.as_ref())
    {
        paymaster_child =
            Some(start_paymaster_sidecar(paymaster_cfg, paymaster_bootstrap, rpc_addr).await?);
    }

    #[cfg(feature = "vrf")]
    if let (Some(vrf_cfg), Some(vrf_bootstrap)) = (&config.vrf, bootstrap.vrf.as_ref()) {
        vrf_child = Some(start_vrf_sidecar(vrf_cfg, vrf_bootstrap).await?);
    }

    #[cfg(feature = "vrf")]
    let processes = SidecarProcesses::new(paymaster_child, vrf_child);
    #[cfg(not(feature = "vrf"))]
    let processes = SidecarProcesses::new(paymaster_child);

    Ok(processes)
}

async fn start_paymaster_sidecar(
    config: &PaymasterSidecarConfig<'_>,
    bootstrap: &PaymasterBootstrap,
    rpc_addr: &SocketAddr,
) -> Result<Child> {
    let bin = config.options.bin.clone().unwrap_or_else(|| "paymaster-service".into());
    let bin = resolve_executable(Path::new(&bin))?;
    let rpc_url = local_rpc_url(rpc_addr);
    let profile = build_paymaster_profile(config, bootstrap, &rpc_url)?;
    let profile_path = write_paymaster_profile(&profile)?;

    let mut command = Command::new(bin);
    command
        .env("PAYMASTER_PROFILE", &profile_path)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);

    info!(target: "sidecar", profile = %profile_path.display(), "paymaster profile generated");

    let child = command.spawn().context("failed to spawn paymaster sidecar")?;

    let url = Url::parse(&format!("http://127.0.0.1:{}", config.port)).expect("valid url");
    wait_for_paymaster_ready(&url, Some(&config.api_key), BOOTSTRAP_TIMEOUT).await?;

    Ok(child)
}

#[cfg(feature = "vrf")]
async fn start_vrf_sidecar(
    config: &VrfSidecarConfig<'_>,
    bootstrap: &katana_node::sidecar::VrfBootstrap,
) -> Result<Child> {
    if config.port != VRF_SERVER_PORT {
        return Err(anyhow!(
            "vrf-server uses a fixed port of {VRF_SERVER_PORT}; set --vrf.port={VRF_SERVER_PORT}"
        ));
    }

    let bin = config.options.bin.clone().unwrap_or_else(|| "vrf-server".into());
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
    wait_for_http_ok(&url, "vrf info", BOOTSTRAP_TIMEOUT).await?;

    Ok(child)
}

fn resolve_executable(path: &Path) -> Result<PathBuf> {
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

#[derive(Debug, Serialize)]
struct PaymasterProfile {
    verbosity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    prometheus: Option<PaymasterPrometheusProfile>,
    rpc: PaymasterRpcProfile,
    forwarder: String,
    supported_tokens: Vec<String>,
    max_fee_multiplier: f32,
    provider_fee_overhead: f32,
    estimate_account: PaymasterAccountProfile,
    gas_tank: PaymasterAccountProfile,
    relayers: PaymasterRelayersProfile,
    starknet: PaymasterStarknetProfile,
    price: PaymasterPriceProfile,
    sponsoring: PaymasterSponsoringProfile,
}

#[derive(Debug, Serialize)]
struct PaymasterPrometheusProfile {
    endpoint: String,
}

#[derive(Debug, Serialize)]
struct PaymasterRpcProfile {
    port: u64,
}

#[derive(Debug, Serialize)]
struct PaymasterAccountProfile {
    address: String,
    private_key: String,
}

#[derive(Debug, Serialize)]
struct PaymasterRelayersProfile {
    private_key: String,
    addresses: Vec<String>,
    min_relayer_balance: String,
    lock: PaymasterLockProfile,
}

#[derive(Debug, Serialize)]
struct PaymasterLockProfile {
    mode: String,
    retry_timeout: u64,
}

#[derive(Debug, Serialize)]
struct PaymasterStarknetProfile {
    chain_id: String,
    endpoint: String,
    timeout: u64,
    fallbacks: Vec<String>,
}

#[derive(Debug, Serialize)]
struct PaymasterPriceProfile {
    provider: String,
    endpoint: String,
    api_key: String,
}

#[derive(Debug, Serialize)]
struct PaymasterSponsoringProfile {
    mode: String,
    api_key: String,
    sponsor_metadata: Vec<Felt>,
}

fn build_paymaster_profile(
    config: &PaymasterSidecarConfig<'_>,
    bootstrap: &PaymasterBootstrap,
    rpc_url: &Url,
) -> Result<PaymasterProfile> {
    use katana_genesis::constant::{DEFAULT_ETH_FEE_TOKEN_ADDRESS, DEFAULT_STRK_FEE_TOKEN_ADDRESS};

    let chain_id = paymaster_chain_id(bootstrap.chain_id)?;
    let price_endpoint = paymaster_price_endpoint(bootstrap.chain_id)?;
    let price_api_key = config.options.price_api_key.clone().unwrap_or_default();

    let eth_token = format_felt(DEFAULT_ETH_FEE_TOKEN_ADDRESS.into());
    let strk_token = format_felt(DEFAULT_STRK_FEE_TOKEN_ADDRESS.into());

    Ok(PaymasterProfile {
        verbosity: "info".to_string(),
        prometheus: None,
        rpc: PaymasterRpcProfile { port: config.port as u64 },
        forwarder: format_felt(bootstrap.forwarder_address.into()),
        supported_tokens: vec![eth_token, strk_token],
        max_fee_multiplier: 3.0,
        provider_fee_overhead: 0.1,
        estimate_account: PaymasterAccountProfile {
            address: format_felt(bootstrap.estimate_account_address.into()),
            private_key: format_felt(bootstrap.estimate_account_private_key),
        },
        gas_tank: PaymasterAccountProfile {
            address: format_felt(bootstrap.gas_tank_address.into()),
            private_key: format_felt(bootstrap.gas_tank_private_key),
        },
        relayers: PaymasterRelayersProfile {
            private_key: format_felt(bootstrap.relayer_private_key),
            addresses: vec![format_felt(bootstrap.relayer_address.into())],
            min_relayer_balance: format_felt(Felt::ZERO),
            lock: PaymasterLockProfile { mode: "seggregated".to_string(), retry_timeout: 5 },
        },
        starknet: PaymasterStarknetProfile {
            chain_id,
            endpoint: rpc_url.to_string(),
            timeout: 30,
            fallbacks: Vec::new(),
        },
        price: PaymasterPriceProfile {
            provider: "avnu".to_string(),
            endpoint: price_endpoint.to_string(),
            api_key: price_api_key,
        },
        sponsoring: PaymasterSponsoringProfile {
            mode: "self".to_string(),
            api_key: config.api_key.clone(),
            sponsor_metadata: Vec::new(),
        },
    })
}

fn write_paymaster_profile(profile: &PaymasterProfile) -> Result<PathBuf> {
    let payload = serde_json::to_string_pretty(profile).context("serialize paymaster profile")?;
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let pid = std::process::id();

    let mut path = env::temp_dir();
    path.push(format!("katana-paymaster-profile-{timestamp}-{pid}.json"));
    fs::write(&path, payload).context("write paymaster profile")?;
    Ok(path)
}

fn local_rpc_url(addr: &SocketAddr) -> Url {
    let host = match addr.ip() {
        std::net::IpAddr::V4(ip) if ip.is_unspecified() => {
            std::net::IpAddr::V4([127, 0, 0, 1].into())
        }
        std::net::IpAddr::V6(ip) if ip.is_unspecified() => {
            std::net::IpAddr::V4([127, 0, 0, 1].into())
        }
        ip => ip,
    };

    Url::parse(&format!("http://{}:{}", host, addr.port())).expect("valid rpc url")
}

fn paymaster_chain_id(chain_id: ChainId) -> Result<String> {
    match chain_id {
        ChainId::Named(NamedChainId::Sepolia) => Ok("sepolia".to_string()),
        ChainId::Named(NamedChainId::Mainnet) => Ok("mainnet".to_string()),
        ChainId::Named(other) => Err(anyhow!(
            "paymaster sidecar only supports SN_MAIN or SN_SEPOLIA chain ids, got {other}"
        )),
        ChainId::Id(id) => {
            Err(anyhow!("paymaster sidecar requires SN_MAIN or SN_SEPOLIA chain id, got {id:#x}"))
        }
    }
}

fn paymaster_price_endpoint(chain_id: ChainId) -> Result<&'static str> {
    match chain_id {
        ChainId::Named(NamedChainId::Sepolia) => Ok(DEFAULT_AVNU_PRICE_SEPOLIA_ENDPOINT),
        ChainId::Named(NamedChainId::Mainnet) => Ok(DEFAULT_AVNU_PRICE_MAINNET_ENDPOINT),
        ChainId::Named(other) => Err(anyhow!(
            "paymaster sidecar only supports SN_MAIN or SN_SEPOLIA chain ids, got {other}"
        )),
        ChainId::Id(id) => {
            Err(anyhow!("paymaster sidecar requires SN_MAIN or SN_SEPOLIA chain id, got {id:#x}"))
        }
    }
}

#[cfg(feature = "vrf")]
async fn wait_for_http_ok(url: &str, name: &str, timeout: Duration) -> Result<()> {
    let client = reqwest::Client::new();
    let start = Instant::now();

    loop {
        match client.get(url).send().await {
            Ok(resp) if resp.status().is_success() => {
                info!(target: "sidecar", %name, "sidecar ready");
                return Ok(());
            }
            Ok(resp) => {
                debug!(target: "sidecar", %name, status = %resp.status(), "waiting for sidecar");
            }
            Err(err) => {
                debug!(target: "sidecar", %name, error = %err, "waiting for sidecar");
            }
        }

        if start.elapsed() > timeout {
            warn!(target: "sidecar", %name, "sidecar did not become ready in time");
            return Err(anyhow!("{} did not become ready before timeout", name));
        }

        sleep(Duration::from_millis(200)).await;
    }
}

async fn wait_for_paymaster_ready(
    url: &Url,
    api_key: Option<&str>,
    timeout: Duration,
) -> Result<()> {
    let client = reqwest::Client::new();
    let start = Instant::now();

    let payload = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "paymaster_health",
        "params": [],
    });

    loop {
        let mut request = client.post(url.as_str()).json(&payload);
        if let Some(key) = api_key {
            request = request.header("x-paymaster-api-key", key);
        }

        match request.send().await {
            Ok(resp) if resp.status().is_success() => {
                match resp.json::<serde_json::Value>().await {
                    Ok(body) => {
                        if body.get("error").is_none() {
                            info!(target: "sidecar", name = "paymaster health", "sidecar ready");
                            return Ok(());
                        }
                        debug!(target: "sidecar", name = "paymaster health", "paymaster not ready yet");
                    }
                    Err(err) => {
                        debug!(target: "sidecar", name = "paymaster health", error = %err, "waiting for sidecar");
                    }
                }
            }
            Ok(resp) => {
                debug!(target: "sidecar", name = "paymaster health", status = %resp.status(), "waiting for sidecar");
            }
            Err(err) => {
                debug!(target: "sidecar", name = "paymaster health", error = %err, "waiting for sidecar");
            }
        }

        if start.elapsed() > timeout {
            warn!(target: "sidecar", name = "paymaster health", "sidecar did not become ready in time");
            return Err(anyhow!("paymaster did not become ready before timeout"));
        }

        sleep(Duration::from_millis(200)).await;
    }
}
