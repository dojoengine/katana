//! Sidecar bootstrap and process management for CLI.
//!
//! This module handles:
//! - Building paymaster and VRF configurations from CLI options
//! - Bootstrapping paymaster and VRF services (deploying contracts)
//! - Spawning and managing sidecar processes when running in sidecar mode
//!
//! The node treats all paymaster/VRF services as external - this module bridges
//! the gap by deploying necessary contracts and spawning sidecar processes.

use std::net::SocketAddr;

use anyhow::{anyhow, Result};
#[cfg(feature = "vrf")]
pub use cartridge::vrf::{
    bootstrap_vrf, derive_vrf_accounts, start_vrf_sidecar, VrfBootstrapConfig, VrfBootstrapResult,
    VrfDerivedAccounts, VrfSidecarConfig, VrfSidecarInfo, VRF_SERVER_PORT,
};
use katana_chain_spec::ChainSpec;
use katana_genesis::allocation::GenesisAccountAlloc;
use katana_genesis::constant::{DEFAULT_ETH_FEE_TOKEN_ADDRESS, DEFAULT_STRK_FEE_TOKEN_ADDRESS};
use katana_node::config::paymaster::PaymasterConfig;
#[cfg(feature = "vrf")]
use katana_node::config::paymaster::VrfConfig;
use katana_node::config::Config;
pub use katana_paymaster::{
    format_felt, wait_for_paymaster_ready, PaymasterService, PaymasterServiceConfig,
    PaymasterServiceConfigBuilder, PaymasterSidecarProcess,
};
use katana_primitives::chain::ChainId;
use katana_primitives::{ContractAddress, Felt};
use tracing::info;
use url::Url;

use crate::options::PaymasterOptions;
#[cfg(feature = "vrf")]
use crate::options::VrfOptions;

/// Default API key for the paymaster sidecar.
pub const DEFAULT_PAYMASTER_API_KEY: &str = "paymaster_katana";

// ============================================================================
// Sidecar Info Types
// ============================================================================

/// Sidecar-specific info for paymaster (used by CLI to start sidecar process).
#[cfg(feature = "paymaster")]
#[derive(Debug, Clone)]
pub struct PaymasterSidecarInfo {
    pub port: u16,
    pub api_key: String,
}

/// Build the VRF configuration from CLI options.
///
/// Returns `None` if VRF is not enabled.
/// Returns `(VrfConfig, Option<VrfSidecarInfo>)` where the sidecar info
/// is `Some` in sidecar mode and `None` in external mode.
///
/// The `rpc_addr` parameter is the address the node's RPC server is bound to,
/// used to construct the RPC URL for the VRF server to query state.
#[cfg(feature = "vrf")]
pub fn build_vrf_config(
    options: &VrfOptions,
) -> Result<Option<(VrfConfig, Option<VrfSidecarInfo>)>> {
    if !options.is_enabled() {
        return Ok(None);
    }

    // Determine mode based on whether URL is provided
    let is_external = options.is_external();

    let (url, sidecar_info) = if is_external {
        // External mode: use the provided URL
        let url = options.url.clone().expect("URL must be set in external mode");
        (url, None)
    } else {
        // Sidecar mode: find a free port dynamically
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let port = addr.port();
        let url = Url::parse(&format!("http://{addr}"))?;
        let sidecar_info = VrfSidecarInfo { port };
        (url, Some(sidecar_info))
    };

    Ok(Some((VrfConfig { url }, sidecar_info)))
}

// ============================================================================
// Bootstrap Types
// ============================================================================

/// Result of bootstrapping sidecars.
#[derive(Debug, Default)]
pub struct BootstrapResult {
    #[cfg(feature = "vrf")]
    pub vrf: Option<VrfBootstrapResult>,
}

/// Paymaster bootstrap info combining config and result.
#[derive(Debug, Clone)]
pub struct PaymasterBootstrapInfo {
    /// Relayer account address.
    pub relayer_address: ContractAddress,
    /// Relayer account private key.
    pub relayer_private_key: Felt,
    /// Gas tank account address.
    pub gas_tank_address: ContractAddress,
    /// Gas tank account private key.
    pub gas_tank_private_key: Felt,
    /// Estimation account address.
    pub estimate_account_address: ContractAddress,
    /// Estimation account private key.
    pub estimate_account_private_key: Felt,
    /// The deployed forwarder contract address.
    pub forwarder_address: ContractAddress,
    /// The chain ID of the network.
    pub chain_id: ChainId,
}

/// Configuration for bootstrapping sidecars.
pub struct BootstrapConfig {
    #[cfg(feature = "vrf")]
    pub vrf: Option<VrfBootstrapConfig>,
    #[allow(dead_code)]
    pub fee_enabled: bool,
}

pub struct PaymasterBootstrapInput {
    /// RPC URL for the katana node.
    pub rpc_url: Url,
}

// ============================================================================
// Bootstrap Functions
// ============================================================================

/// Bootstrap sidecars by deploying necessary contracts and preparing configuration.
///
/// This must be called after the node is launched but before sidecars are started.
pub async fn bootstrap_sidecars(config: &BootstrapConfig) -> Result<BootstrapResult> {
    let mut result = BootstrapResult::default();

    #[cfg(feature = "vrf")]
    if let Some(vrf_cfg) = &config.vrf {
        let bootstrap = bootstrap_vrf(vrf_cfg).await?;
        result.vrf = Some(bootstrap);
    }

    Ok(result)
}

pub async fn bootstrap_paymaster(
    options: &PaymasterOptions,
    paymaster_url: Url,
    rpc_url: SocketAddr,
    chain: &ChainSpec,
) -> Result<PaymasterService> {
    let (relayer_addr, relayer_pk) = prefunded_account(chain, 0)?;
    let (gas_tank_addr, gas_tank_pk) = prefunded_account(chain, 1)?;
    let (estimate_account_addr, estimate_account_pk) = prefunded_account(chain, 2)?;

    let port = paymaster_url.port().unwrap();

    let mut builder = PaymasterServiceConfigBuilder::new()
        .rpc(rpc_url)
        .port(port)
        .api_key(DEFAULT_PAYMASTER_API_KEY)
        .relayer(relayer_addr, relayer_pk)
        .gas_tank(gas_tank_addr, gas_tank_pk)
        .estimate_account(estimate_account_addr, estimate_account_pk)
        .tokens(DEFAULT_ETH_FEE_TOKEN_ADDRESS, DEFAULT_STRK_FEE_TOKEN_ADDRESS);

    if let Some(bin) = &options.bin {
        builder = builder.program_path(bin.clone());
    }

    let mut paymaster = PaymasterService::new(builder.build().await?);
    paymaster.bootstrap().await?;

    Ok(paymaster)
}

fn prefunded_account(chain_spec: &ChainSpec, index: u16) -> Result<(ContractAddress, Felt)> {
    let (address, allocation) = chain_spec
        .genesis()
        .accounts()
        .nth(index as usize)
        .ok_or_else(|| anyhow!("prefunded account index {} out of range", index))?;

    let private_key = match allocation {
        GenesisAccountAlloc::DevAccount(account) => account.private_key,
        _ => return Err(anyhow!("prefunded account {} has no private key", address)),
    };

    Ok((*address, private_key))
}

// ============================================================================
// Sidecar Process Management
// ============================================================================

/// Manages sidecar child processes.
///
/// When dropped, the sidecar processes are killed.
#[derive(Debug)]
pub struct SidecarProcesses {
    paymaster: Option<PaymasterSidecarProcess>,
    #[cfg(feature = "vrf")]
    vrf: Option<tokio::process::Child>,
}

impl SidecarProcesses {
    #[cfg(feature = "vrf")]
    pub fn new(
        paymaster: Option<PaymasterSidecarProcess>,
        vrf: Option<tokio::process::Child>,
    ) -> Self {
        Self { paymaster, vrf }
    }

    #[cfg(not(feature = "vrf"))]
    pub fn new(paymaster: Option<PaymasterSidecarProcess>) -> Self {
        Self { paymaster }
    }

    /// Gracefully shutdown all sidecar processes.
    ///
    /// This kills each process and waits for it to exit.
    pub async fn shutdown(&mut self) {
        if let Some(ref mut process) = self.paymaster {
            info!(target: "sidecar", "shutting down paymaster sidecar");
            let _ = process.shutdown().await;
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
        if let Some(mut process) = self.paymaster.take() {
            let _ = process.process().start_kill();
        }
        #[cfg(feature = "vrf")]
        if let Some(mut child) = self.vrf.take() {
            let _ = child.start_kill();
        }
    }
}

/// Configuration for starting sidecars.
pub struct SidecarStartConfig<'a> {
    #[cfg(feature = "vrf")]
    pub vrf: Option<VrfStartConfig<'a>>,
}

/// Configuration for starting the paymaster sidecar.
pub struct PaymasterStartConfig<'a> {
    pub options: &'a PaymasterOptions,
    pub port: u16,
    pub api_key: String,
    pub rpc_url: Url,
}

/// Configuration for starting the VRF sidecar.
#[cfg(feature = "vrf")]
pub struct VrfStartConfig<'a> {
    pub options: &'a VrfOptions,
    pub port: u16,
}

/// Start sidecar processes using the bootstrap data.
pub async fn start_sidecars(
    config: &SidecarStartConfig<'_>,
    bootstrap: &BootstrapResult,
) -> Result<SidecarProcesses> {
    let mut paymaster_process = None;
    #[cfg(feature = "vrf")]
    let mut vrf_child = None;

    if let (Some(paymaster_cfg), Some(paymaster_bootstrap)) =
        (&config.paymaster, bootstrap.paymaster.as_ref())
    {
        // Build config using the builder pattern (unchecked since accounts are from genesis)
        let mut builder = PaymasterServiceConfigBuilder::new()
            .rpc(paymaster_cfg.rpc_url.clone())
            .port(paymaster_cfg.port)
            .api_key(paymaster_cfg.api_key.clone())
            .relayer(paymaster_bootstrap.relayer_address, paymaster_bootstrap.relayer_private_key)
            .gas_tank(
                paymaster_bootstrap.gas_tank_address,
                paymaster_bootstrap.gas_tank_private_key,
            )
            .estimate_account(
                paymaster_bootstrap.estimate_account_address,
                paymaster_bootstrap.estimate_account_private_key,
            )
            .tokens(DEFAULT_ETH_FEE_TOKEN_ADDRESS, DEFAULT_STRK_FEE_TOKEN_ADDRESS);

        // Set optional fields on builder
        if let Some(bin) = &paymaster_cfg.options.bin {
            builder = builder.program_path(bin.clone());
        }
        if let Some(price_api_key) = &paymaster_cfg.options.price_api_key {
            builder = builder.price_api_key(price_api_key.clone());
        }

        let paymaster_config = builder.build_unchecked()?;

        // Create sidecar with forwarder and chain_id from bootstrap
        let sidecar = PaymasterService::new(paymaster_config)
            .forwarder(paymaster_bootstrap.forwarder_address)
            .chain_id(paymaster_bootstrap.chain_id);

        paymaster_process = Some(sidecar.start().await?);
    }

    #[cfg(feature = "vrf")]
    if let (Some(vrf_cfg), Some(vrf_bootstrap)) = (&config.vrf, bootstrap.vrf.as_ref()) {
        let sidecar_config =
            VrfSidecarConfig { bin: vrf_cfg.options.bin.clone(), port: vrf_cfg.port };
        vrf_child = Some(start_vrf_sidecar(&sidecar_config, vrf_bootstrap).await?);
    }

    #[cfg(feature = "vrf")]
    let processes = SidecarProcesses::new(paymaster_process, vrf_child);
    #[cfg(not(feature = "vrf"))]
    let processes = SidecarProcesses::new(paymaster_process);

    Ok(processes)
}

/// Helper to construct the local RPC URL from the socket address.
pub fn local_rpc_url(addr: &SocketAddr) -> Url {
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

// ============================================================================
// High-Level Bootstrap and Start API
// ============================================================================

/// Bootstrap contracts and start sidecar processes if needed.
///
/// This function is called after the node is launched to:
/// 1. Bootstrap necessary contracts (forwarder, VRF accounts)
/// 2. Start sidecar processes in sidecar mode
///
/// Returns `None` if no sidecars need to be started.
#[cfg(feature = "cartridge")]
pub async fn bootstrap_and_start_sidecars(
    paymaster_sidecar: Option<&PaymasterSidecarInfo>,
    paymaster_options: &PaymasterOptions,
    #[cfg(feature = "vrf")] vrf_options: &VrfOptions,
    chain_spec: &ChainSpec,
    rpc_addr: &SocketAddr,
    #[cfg(feature = "vrf")] vrf_sidecar: Option<&VrfSidecarInfo>,
    fee_enabled: bool,
) -> Result<Option<SidecarProcesses>> {
    // Build RPC URL for paymaster bootstrap
    let rpc_url = local_rpc_url(rpc_addr);

    // Build bootstrap config
    #[cfg(feature = "vrf")]
    let vrf_bootstrap_config = if vrf_sidecar.is_some() {
        // Always use the first genesis account (index 0)
        let (source_address, source_private_key) = prefunded_account(chain_spec, 0)?;
        Some(VrfBootstrapConfig {
            rpc_url: rpc_url.clone(),
            source_address,
            source_private_key,
            fund_account: fee_enabled,
        })
    } else {
        None
    };

    let bootstrap_config = BootstrapConfig {
        #[cfg(feature = "vrf")]
        vrf: vrf_bootstrap_config,
        fee_enabled,
    };

    // Bootstrap contracts
    let bootstrap = bootstrap_sidecars(&bootstrap_config).await?;

    // Build sidecar start config
    let paymaster_start_config = paymaster_sidecar.map(|info| PaymasterStartConfig {
        options: paymaster_options,
        port: info.port,
        api_key: info.api_key.clone(),
        rpc_url: rpc_url.clone(),
    });

    #[cfg(feature = "vrf")]
    let vrf_config =
        vrf_sidecar.map(|info| VrfStartConfig { options: vrf_options, port: info.port });

    let start_config = SidecarStartConfig {
        #[cfg(feature = "vrf")]
        vrf: vrf_config,
    };

    // Start sidecar processes
    let processes = start_sidecars(&start_config, &bootstrap).await?;
    Ok(Some(processes))
}
