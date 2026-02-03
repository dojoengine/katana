//! Paymaster sidecar bootstrap and process management.
//!
//! This crate handles:
//! - Bootstrapping the paymaster service (deploying forwarder contract via RPC)
//! - Spawning and managing the paymaster sidecar process
//! - Generating paymaster configuration profiles
//!
//! This crate uses the starknet crate's account abstraction for transaction handling.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::{env, fs};

use anyhow::{anyhow, Context, Result};
use katana_primitives::chain::{ChainId, NamedChainId};
use katana_primitives::utils::get_contract_address;
use katana_primitives::{ContractAddress, Felt};
use serde::Serialize;
use starknet::accounts::{Account, ExecutionEncoding, SingleOwnerAccount};
use starknet::contract::ContractFactory;
use starknet::core::types::{BlockId, BlockTag, Call, StarknetError};
use starknet::macros::selector;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider, ProviderError};
use starknet::signers::{LocalWallet, SigningKey};
use tokio::process::{Child, Command};
use tokio::time::sleep;
use tracing::{debug, info, warn};
use url::Url;

// ============================================================================
// Constants
// ============================================================================

const FORWARDER_SALT: u64 = 0x12345;
const BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_AVNU_PRICE_MAINNET_ENDPOINT: &str = "https://starknet.impulse.avnu.fi/v3/";

// ============================================================================
// Bootstrap Configuration Types
// ============================================================================

/// Bootstrap configuration for the paymaster service.
/// These are the input parameters needed to perform bootstrap.
#[derive(Debug, Clone)]
pub struct PaymasterBootstrapConfig {
    /// RPC URL of the katana node.
    pub rpc_url: Url,

    /// Relayer account address (prefunded account).
    pub relayer_address: ContractAddress,
    /// Relayer account private key.
    pub relayer_private_key: Felt,

    /// Gas tank account address (prefunded account).
    pub gas_tank_address: ContractAddress,
    /// Gas tank account private key.
    pub gas_tank_private_key: Felt,

    /// Estimation account address (prefunded account).
    pub estimate_account_address: ContractAddress,
    /// Estimation account private key.
    pub estimate_account_private_key: Felt,
}

/// Result of bootstrap operations.
#[derive(Debug, Clone)]
pub struct PaymasterBootstrapResult {
    /// The deployed forwarder contract address.
    pub forwarder_address: ContractAddress,
    /// The chain ID of the network.
    pub chain_id: ChainId,
}

/// Configuration for the paymaster sidecar process.
#[derive(Debug, Clone)]
pub struct PaymasterSidecarConfig {
    /// Path to the paymaster-service binary, or None to look up in PATH.
    pub program_path: Option<PathBuf>,

    /// Port for the paymaster service.
    pub port: u16,
    /// API key for the paymaster service.
    pub api_key: String,
    /// Price API key (for AVNU price feed).
    pub price_api_key: Option<String>,

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

    /// Forwarder contract address (from bootstrap result).
    pub forwarder_address: ContractAddress,

    /// Chain ID (from bootstrap result).
    pub chain_id: ChainId,
    /// RPC URL of the katana node.
    pub rpc_url: Url,

    /// ETH token contract address.
    pub eth_token_address: ContractAddress,
    /// STRK token contract address.
    pub strk_token_address: ContractAddress,
}

// ============================================================================
// Bootstrap Functions
// ============================================================================

/// Bootstrap the paymaster by deploying the forwarder contract via RPC.
///
/// This function:
/// 1. Connects to the node via RPC
/// 2. Gets the chain ID
/// 3. Computes the deterministic forwarder address
/// 4. Deploys the forwarder if not already deployed
/// 5. Whitelists the relayer address
pub async fn bootstrap_paymaster(
    config: &PaymasterBootstrapConfig,
) -> Result<PaymasterBootstrapResult> {
    let provider = Arc::new(JsonRpcClient::new(HttpTransport::new(config.rpc_url.clone())));

    // Get chain ID from the node
    let chain_id_felt = provider.chain_id().await.context("failed to get chain ID from node")?;
    let chain_id = ChainId::Id(chain_id_felt);

    let forwarder_class_hash = avnu_forwarder_class_hash()?;
    // When using UDC with unique=0 (non-unique deployment), the deployer_address
    // used in address computation is 0, not the actual deployer or UDC address.
    let forwarder_address = get_contract_address(
        Felt::from(FORWARDER_SALT),
        forwarder_class_hash,
        &[config.relayer_address.into(), config.gas_tank_address.into()],
        Felt::ZERO,
    )
    .into();

    // Create the relayer account for transactions
    let signer = LocalWallet::from(SigningKey::from_secret_scalar(config.relayer_private_key));
    let mut account = SingleOwnerAccount::new(
        provider.clone(),
        signer,
        config.relayer_address.into(),
        chain_id_felt,
        ExecutionEncoding::New,
    );
    account.set_block_id(BlockId::Tag(BlockTag::PreConfirmed));

    // Deploy forwarder if not already deployed
    if !is_deployed(&provider, forwarder_address).await? {
        #[allow(deprecated)]
        let factory = ContractFactory::new(forwarder_class_hash, &account);
        factory
            .deploy_v3(
                vec![config.relayer_address.into(), config.gas_tank_address.into()],
                Felt::from(FORWARDER_SALT),
                false,
            )
            .send()
            .await
            .map_err(|e| anyhow!("failed to deploy forwarder: {e}"))?;

        wait_for_contract(&provider, forwarder_address, BOOTSTRAP_TIMEOUT).await?;
    }

    // Whitelist the relayer
    let whitelist_call = Call {
        to: forwarder_address.into(),
        selector: selector!("set_whitelisted_address"),
        calldata: vec![config.relayer_address.into(), Felt::ONE],
    };

    account
        .execute_v3(vec![whitelist_call])
        .send()
        .await
        .map_err(|e| anyhow!("failed to whitelist relayer: {e}"))?;

    Ok(PaymasterBootstrapResult { forwarder_address, chain_id })
}

async fn is_deployed(
    provider: &JsonRpcClient<HttpTransport>,
    address: ContractAddress,
) -> Result<bool> {
    let address_felt: Felt = address.into();
    match provider.get_class_hash_at(BlockId::Tag(BlockTag::PreConfirmed), address_felt).await {
        Ok(_) => Ok(true),
        Err(ProviderError::StarknetError(StarknetError::ContractNotFound)) => Ok(false),
        Err(e) => Err(anyhow!("failed to check contract deployment: {e}")),
    }
}

async fn wait_for_contract(
    provider: &JsonRpcClient<HttpTransport>,
    address: ContractAddress,
    timeout: Duration,
) -> Result<()> {
    let start = Instant::now();
    loop {
        if is_deployed(provider, address).await? {
            return Ok(());
        }

        if start.elapsed() > timeout {
            return Err(anyhow!("contract {address} not deployed before timeout"));
        }

        sleep(Duration::from_millis(200)).await;
    }
}

fn avnu_forwarder_class_hash() -> Result<Felt> {
    let class = katana_primitives::utils::class::parse_sierra_class(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../controller/classes/avnu_Forwarder.contract_class.json"
    )))?;
    class.class_hash().context("failed to compute forwarder class hash")
}

// ============================================================================
// Sidecar Process Management
// ============================================================================

/// Start the paymaster sidecar process.
///
/// This spawns the paymaster-service binary with the appropriate configuration.
pub async fn start_paymaster_sidecar(config: &PaymasterSidecarConfig) -> Result<Child> {
    let bin = config.program_path.clone().unwrap_or_else(|| PathBuf::from("paymaster-service"));
    let bin = resolve_executable(&bin)?;
    let profile = build_paymaster_profile(config)?;
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

// ============================================================================
// Paymaster Profile
// ============================================================================

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

fn build_paymaster_profile(config: &PaymasterSidecarConfig) -> Result<PaymasterProfile> {
    let chain_id = paymaster_chain_id(config.chain_id)?;
    let price_endpoint = DEFAULT_AVNU_PRICE_MAINNET_ENDPOINT;
    let price_api_key = config.price_api_key.clone().unwrap_or_default();

    let eth_token = format_felt(config.eth_token_address.into());
    let strk_token = format_felt(config.strk_token_address.into());

    Ok(PaymasterProfile {
        verbosity: "info".to_string(),
        prometheus: None,
        rpc: PaymasterRpcProfile { port: config.port as u64 },
        forwarder: format_felt(config.forwarder_address.into()),
        supported_tokens: vec![eth_token, strk_token],
        max_fee_multiplier: 3.0,
        provider_fee_overhead: 0.1,
        estimate_account: PaymasterAccountProfile {
            address: format_felt(config.estimate_account_address.into()),
            private_key: format_felt(config.estimate_account_private_key),
        },
        gas_tank: PaymasterAccountProfile {
            address: format_felt(config.gas_tank_address.into()),
            private_key: format_felt(config.gas_tank_private_key),
        },
        relayers: PaymasterRelayersProfile {
            private_key: format_felt(config.relayer_private_key),
            addresses: vec![format_felt(config.relayer_address.into())],
            min_relayer_balance: format_felt(Felt::ZERO),
            lock: PaymasterLockProfile { mode: "seggregated".to_string(), retry_timeout: 5 },
        },
        starknet: PaymasterStarknetProfile {
            chain_id,
            endpoint: config.rpc_url.to_string(),
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

fn paymaster_chain_id(chain_id: ChainId) -> Result<String> {
    match chain_id {
        ChainId::Named(NamedChainId::Mainnet) => Ok("mainnet".to_string()),
        _ => Ok("sepolia".to_string()),
    }
}

/// Wait for the paymaster sidecar to become ready.
pub async fn wait_for_paymaster_ready(
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

/// Format a Felt as a hex string with 0x prefix.
pub fn format_felt(value: Felt) -> String {
    format!("{value:#x}")
}
