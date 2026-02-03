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
use katana_core::backend::Backend;
use katana_executor::ExecutorFactory;
use katana_genesis::allocation::GenesisAccountAlloc;
use katana_genesis::constant::{DEFAULT_ETH_FEE_TOKEN_ADDRESS, DEFAULT_STRK_FEE_TOKEN_ADDRESS};
#[cfg(feature = "vrf")]
use katana_node::config::paymaster::{VrfConfig, VrfKeySource as NodeVrfKeySource};
pub use katana_paymaster::{
    bootstrap_paymaster, format_felt, start_paymaster_sidecar, wait_for_paymaster_ready,
    PaymasterBootstrapConfig, PaymasterBootstrapResult, PaymasterSidecarConfig,
};
use katana_primitives::chain::ChainId;
use katana_primitives::{ContractAddress, Felt};
use katana_provider::ProviderFactory;
use tokio::process::Child;
use tracing::info;
use url::Url;

use crate::options::PaymasterOptions;
#[cfg(feature = "vrf")]
use crate::options::{VrfKeySource as OptionsVrfKeySource, VrfOptions};

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
    rpc_addr: Option<SocketAddr>,
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
        // Sidecar mode: use configured port (VRF server uses fixed port 3000)
        let port = options.port;
        let url = Url::parse(&format!("http://127.0.0.1:{port}")).expect("valid url");
        let sidecar_info = VrfSidecarInfo { port };
        (url, Some(sidecar_info))
    };

    let key_source = match options.key_source {
        OptionsVrfKeySource::Prefunded => NodeVrfKeySource::Prefunded,
        OptionsVrfKeySource::Sequencer => NodeVrfKeySource::Sequencer,
    };

    // Construct RPC URL for VRF server to query state
    let rpc_url =
        rpc_addr.map(|addr| Url::parse(&format!("http://{addr}"))).transpose().expect("valid URL");

    let config = VrfConfig { url, key_source, prefunded_index: options.prefunded_index, rpc_url };

    Ok(Some((config, sidecar_info)))
}

// ============================================================================
// Bootstrap Types
// ============================================================================

/// Result of bootstrapping sidecars.
#[derive(Debug, Default)]
pub struct BootstrapResult {
    pub paymaster: Option<PaymasterBootstrapInfo>,
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
    pub paymaster: Option<PaymasterBootstrapInput>,
    #[cfg(feature = "vrf")]
    pub vrf: Option<VrfBootstrapConfig>,
    #[allow(dead_code)]
    pub fee_enabled: bool,
}

/// Input for paymaster bootstrap (extracted from genesis accounts).
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
pub async fn bootstrap_sidecars<EF, PF>(
    config: &BootstrapConfig,
    backend: &Backend<EF, PF>,
) -> Result<BootstrapResult>
where
    EF: ExecutorFactory,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: katana_provider::ProviderRO,
    <PF as ProviderFactory>::ProviderMut: katana_provider::ProviderRW,
{
    let mut result = BootstrapResult::default();

    if let Some(paymaster_input) = &config.paymaster {
        let bootstrap = bootstrap_paymaster_from_genesis(paymaster_input, backend).await?;
        result.paymaster = Some(bootstrap);
    }

    #[cfg(feature = "vrf")]
    if let Some(vrf_cfg) = &config.vrf {
        let bootstrap = bootstrap_vrf(vrf_cfg).await?;
        result.vrf = Some(bootstrap);
    }

    Ok(result)
}

/// Bootstrap the paymaster using genesis accounts from the backend.
///
/// Always uses accounts 0, 1, 2 from genesis for relayer, gas tank, and estimate account.
async fn bootstrap_paymaster_from_genesis<EF, PF>(
    input: &PaymasterBootstrapInput,
    backend: &Backend<EF, PF>,
) -> Result<PaymasterBootstrapInfo>
where
    EF: ExecutorFactory,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: katana_provider::ProviderRO,
    <PF as ProviderFactory>::ProviderMut: katana_provider::ProviderRW,
{
    // Extract account info from genesis - always use accounts 0, 1, 2
    let (relayer_address, relayer_private_key) = prefunded_account(backend, 0)?;
    let (gas_tank_address, gas_tank_private_key) = prefunded_account(backend, 1)?;
    let (estimate_account_address, estimate_account_private_key) = prefunded_account(backend, 2)?;

    // Build bootstrap config for paymaster crate
    let bootstrap_config = PaymasterBootstrapConfig {
        rpc_url: input.rpc_url.clone(),
        relayer_address,
        relayer_private_key,
        gas_tank_address,
        gas_tank_private_key,
        estimate_account_address,
        estimate_account_private_key,
    };

    // Call the paymaster crate's bootstrap function (uses RPC)
    let bootstrap_result = bootstrap_paymaster(&bootstrap_config).await?;

    Ok(PaymasterBootstrapInfo {
        relayer_address,
        relayer_private_key,
        gas_tank_address,
        gas_tank_private_key,
        estimate_account_address,
        estimate_account_private_key,
        forwarder_address: bootstrap_result.forwarder_address,
        chain_id: bootstrap_result.chain_id,
    })
}

fn prefunded_account<EF, PF>(
    backend: &Backend<EF, PF>,
    index: u16,
) -> Result<(ContractAddress, Felt)>
where
    EF: ExecutorFactory,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: katana_provider::ProviderRO,
    <PF as ProviderFactory>::ProviderMut: katana_provider::ProviderRW,
{
    let (address, allocation) = backend
        .chain_spec
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
    pub paymaster: Option<PaymasterStartConfig<'a>>,
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
    let mut paymaster_child = None;
    #[cfg(feature = "vrf")]
    let mut vrf_child = None;

    if let (Some(paymaster_cfg), Some(paymaster_bootstrap)) =
        (&config.paymaster, bootstrap.paymaster.as_ref())
    {
        let sidecar_config = PaymasterSidecarConfig {
            program_path: paymaster_cfg.options.bin.clone(),
            port: paymaster_cfg.port,
            api_key: paymaster_cfg.api_key.clone(),
            price_api_key: paymaster_cfg.options.price_api_key.clone(),
            relayer_address: paymaster_bootstrap.relayer_address,
            relayer_private_key: paymaster_bootstrap.relayer_private_key,
            gas_tank_address: paymaster_bootstrap.gas_tank_address,
            gas_tank_private_key: paymaster_bootstrap.gas_tank_private_key,
            estimate_account_address: paymaster_bootstrap.estimate_account_address,
            estimate_account_private_key: paymaster_bootstrap.estimate_account_private_key,
            forwarder_address: paymaster_bootstrap.forwarder_address,
            chain_id: paymaster_bootstrap.chain_id,
            rpc_url: paymaster_cfg.rpc_url.clone(),
            eth_token_address: DEFAULT_ETH_FEE_TOKEN_ADDRESS,
            strk_token_address: DEFAULT_STRK_FEE_TOKEN_ADDRESS,
        };
        paymaster_child = Some(start_paymaster_sidecar(&sidecar_config).await?);
    }

    #[cfg(feature = "vrf")]
    if let (Some(vrf_cfg), Some(vrf_bootstrap)) = (&config.vrf, bootstrap.vrf.as_ref()) {
        let sidecar_config =
            VrfSidecarConfig { bin: vrf_cfg.options.bin.clone(), port: vrf_cfg.port };
        vrf_child = Some(start_vrf_sidecar(&sidecar_config, vrf_bootstrap).await?);
    }

    #[cfg(feature = "vrf")]
    let processes = SidecarProcesses::new(paymaster_child, vrf_child);
    #[cfg(not(feature = "vrf"))]
    let processes = SidecarProcesses::new(paymaster_child);

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
pub async fn bootstrap_and_start_sidecars<EF, PF>(
    paymaster_sidecar: Option<&PaymasterSidecarInfo>,
    paymaster_options: &PaymasterOptions,
    #[cfg(feature = "vrf")] vrf_options: &VrfOptions,
    backend: &Backend<EF, PF>,
    rpc_addr: &SocketAddr,
    #[cfg(feature = "vrf")] vrf_sidecar: Option<&VrfSidecarInfo>,
    fee_enabled: bool,
) -> Result<Option<SidecarProcesses>>
where
    EF: ExecutorFactory,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: katana_provider::ProviderRO,
    <PF as ProviderFactory>::ProviderMut: katana_provider::ProviderRW,
{
    // Build RPC URL for paymaster bootstrap
    let rpc_url = local_rpc_url(rpc_addr);

    // Build bootstrap config
    #[cfg(feature = "vrf")]
    let vrf_bootstrap_config = if vrf_sidecar.is_some() {
        // Determine source account for VRF bootstrap based on key source
        let (source_address, source_private_key) = match vrf_options.key_source {
            OptionsVrfKeySource::Prefunded => {
                prefunded_account(backend, vrf_options.prefunded_index)?
            }
            OptionsVrfKeySource::Sequencer => {
                // For sequencer mode, use the sequencer's prefunded account (index 0 by default)
                // The sequencer_address is typically the first prefunded account
                prefunded_account(backend, 0)?
            }
        };
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
        paymaster: paymaster_sidecar.map(|_| PaymasterBootstrapInput { rpc_url: rpc_url.clone() }),
        #[cfg(feature = "vrf")]
        vrf: vrf_bootstrap_config,
        fee_enabled,
    };

    // Bootstrap contracts
    let bootstrap = bootstrap_sidecars(&bootstrap_config, backend).await?;

    // Build sidecar start config
    let paymaster_config = paymaster_sidecar.map(|info| PaymasterStartConfig {
        options: paymaster_options,
        port: info.port,
        api_key: info.api_key.clone(),
        rpc_url: rpc_url.clone(),
    });

    #[cfg(feature = "vrf")]
    let vrf_config =
        vrf_sidecar.map(|info| VrfStartConfig { options: vrf_options, port: info.port });

    let start_config = SidecarStartConfig {
        paymaster: paymaster_config,
        #[cfg(feature = "vrf")]
        vrf: vrf_config,
    };

    // Start sidecar processes
    let processes = start_sidecars(&start_config, &bootstrap).await?;
    Ok(Some(processes))
}
