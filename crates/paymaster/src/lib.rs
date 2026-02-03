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
// Paymaster Sidecar Types
// ============================================================================

/// A running paymaster sidecar process with its configuration.
#[derive(Debug)]
pub struct PaymasterSidecarProcess {
    /// The child process handle.
    process: Child,
    /// The configuration used to start the sidecar.
    config: PaymasterSidecarConfig,
}

impl PaymasterSidecarProcess {
    /// Get the child process handle.
    pub fn process(&mut self) -> &mut Child {
        &mut self.process
    }

    /// Get the sidecar configuration.
    pub fn config(&self) -> &PaymasterSidecarConfig {
        &self.config
    }

    /// Gracefully shutdown the sidecar process.
    pub async fn shutdown(&mut self) -> Result<()> {
        self.process.kill().await?;
        Ok(())
    }
}

/// The resolved configuration for a paymaster sidecar.
#[derive(Debug, Clone)]
pub struct PaymasterSidecarConfig {
    /// Port for the paymaster service.
    pub port: u16,
    /// API key for the paymaster service.
    pub api_key: String,
    /// RPC URL of the katana node.
    pub rpc_url: Url,
    /// Forwarder contract address.
    pub forwarder_address: ContractAddress,
    /// Chain ID.
    pub chain_id: ChainId,
    /// Path to the paymaster-service binary, or None to look up in PATH.
    pub program_path: Option<PathBuf>,
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
    /// ETH token contract address.
    pub eth_token_address: ContractAddress,
    /// STRK token contract address.
    pub strk_token_address: ContractAddress,
}

/// Builder for configuring and starting the paymaster sidecar.
#[derive(Debug, Clone)]
pub struct PaymasterSidecar {
    // Runtime configuration
    program_path: Option<PathBuf>,
    port: u16,
    api_key: String,
    price_api_key: Option<String>,
    rpc_url: Url,

    // Account credentials
    relayer_address: ContractAddress,
    relayer_private_key: Felt,
    gas_tank_address: ContractAddress,
    gas_tank_private_key: Felt,
    estimate_account_address: ContractAddress,
    estimate_account_private_key: Felt,

    // Token addresses
    eth_token_address: ContractAddress,
    strk_token_address: ContractAddress,

    // Bootstrap-derived (can be set directly or via bootstrap)
    forwarder_address: Option<ContractAddress>,
    /// The chain ID (set via `chain_id()` or `bootstrap()`).
    pub chain_id: Option<ChainId>,
}

impl PaymasterSidecar {
    /// Create a new builder with required configuration.
    pub fn new(rpc_url: Url, port: u16, api_key: String) -> Self {
        Self {
            rpc_url,
            port,
            api_key,
            program_path: None,
            price_api_key: None,
            relayer_address: ContractAddress::default(),
            relayer_private_key: Felt::ZERO,
            gas_tank_address: ContractAddress::default(),
            gas_tank_private_key: Felt::ZERO,
            estimate_account_address: ContractAddress::default(),
            estimate_account_private_key: Felt::ZERO,
            eth_token_address: ContractAddress::default(),
            strk_token_address: ContractAddress::default(),
            forwarder_address: None,
            chain_id: None,
        }
    }

    /// Set the path to the paymaster-service binary.
    pub fn program_path(mut self, path: PathBuf) -> Self {
        self.program_path = Some(path);
        self
    }

    /// Set the price API key for AVNU price feed.
    pub fn price_api_key(mut self, key: String) -> Self {
        self.price_api_key = Some(key);
        self
    }

    /// Set relayer account credentials.
    pub fn relayer(mut self, address: ContractAddress, private_key: Felt) -> Self {
        self.relayer_address = address;
        self.relayer_private_key = private_key;
        self
    }

    /// Set gas tank account credentials.
    pub fn gas_tank(mut self, address: ContractAddress, private_key: Felt) -> Self {
        self.gas_tank_address = address;
        self.gas_tank_private_key = private_key;
        self
    }

    /// Set estimation account credentials.
    pub fn estimate_account(mut self, address: ContractAddress, private_key: Felt) -> Self {
        self.estimate_account_address = address;
        self.estimate_account_private_key = private_key;
        self
    }

    /// Set token addresses.
    pub fn tokens(mut self, eth: ContractAddress, strk: ContractAddress) -> Self {
        self.eth_token_address = eth;
        self.strk_token_address = strk;
        self
    }

    /// Set forwarder address directly (skip deploying during bootstrap).
    pub fn forwarder(mut self, address: ContractAddress) -> Self {
        self.forwarder_address = Some(address);
        self
    }

    /// Set chain ID directly (skip fetching from node during bootstrap).
    pub fn chain_id(mut self, chain_id: ChainId) -> Self {
        self.chain_id = Some(chain_id);
        self
    }

    /// Bootstrap the paymaster by deploying the forwarder contract and whitelisting the relayer.
    ///
    /// This method:
    /// 1. Connects to the node via RPC
    /// 2. Gets the chain ID (if not already set)
    /// 3. Computes the deterministic forwarder address
    /// 4. Deploys the forwarder if not already deployed
    /// 5. Whitelists the relayer address
    ///
    /// After calling this method, `forwarder_address` and `chain_id` will be set.
    pub async fn bootstrap(&mut self) -> Result<ContractAddress> {
        let provider = Arc::new(JsonRpcClient::new(HttpTransport::new(self.rpc_url.clone())));

        // Get chain ID if not already set
        let chain_id_felt = if let Some(chain_id) = &self.chain_id {
            chain_id.id()
        } else {
            let chain_id_felt =
                provider.chain_id().await.context("failed to get chain ID from node")?;
            self.chain_id = Some(ChainId::Id(chain_id_felt));
            chain_id_felt
        };

        let forwarder_class_hash = avnu_forwarder_class_hash()?;
        // When using UDC with unique=0 (non-unique deployment), the deployer_address
        // used in address computation is 0, not the actual deployer or UDC address.
        let forwarder_address = get_contract_address(
            Felt::from(FORWARDER_SALT),
            forwarder_class_hash,
            &[self.relayer_address.into(), self.gas_tank_address.into()],
            Felt::ZERO,
        )
        .into();

        // Create the relayer account for transactions
        let signer = LocalWallet::from(SigningKey::from_secret_scalar(self.relayer_private_key));
        let mut account = SingleOwnerAccount::new(
            provider.clone(),
            signer,
            self.relayer_address.into(),
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
                    vec![self.relayer_address.into(), self.gas_tank_address.into()],
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
            calldata: vec![self.relayer_address.into(), Felt::ONE],
        };

        account
            .execute_v3(vec![whitelist_call])
            .send()
            .await
            .map_err(|e| anyhow!("failed to whitelist relayer: {e}"))?;

        self.forwarder_address = Some(forwarder_address);
        Ok(forwarder_address)
    }

    /// Start the paymaster sidecar process.
    ///
    /// Requires `forwarder_address` and `chain_id` to be set (either via builder methods
    /// or by calling `bootstrap()` first).
    ///
    /// Returns a wrapper containing the process handle and resolved configuration.
    pub async fn start(self) -> Result<PaymasterSidecarProcess> {
        let forwarder_address = self.forwarder_address.ok_or_else(|| {
            anyhow!("forwarder_address not set - call bootstrap() or forwarder()")
        })?;
        let chain_id = self
            .chain_id
            .ok_or_else(|| anyhow!("chain_id not set - call bootstrap() or chain_id()"))?;

        // Build the resolved config
        let config = PaymasterSidecarConfig {
            port: self.port,
            api_key: self.api_key,
            rpc_url: self.rpc_url,
            forwarder_address,
            chain_id,
            program_path: self.program_path,
            price_api_key: self.price_api_key,
            relayer_address: self.relayer_address,
            relayer_private_key: self.relayer_private_key,
            gas_tank_address: self.gas_tank_address,
            gas_tank_private_key: self.gas_tank_private_key,
            estimate_account_address: self.estimate_account_address,
            estimate_account_private_key: self.estimate_account_private_key,
            eth_token_address: self.eth_token_address,
            strk_token_address: self.strk_token_address,
        };

        // Build profile and spawn process
        let bin = config.program_path.clone().unwrap_or_else(|| PathBuf::from("paymaster-service"));
        let bin = resolve_executable(&bin)?;
        let profile = build_paymaster_profile(&config)?;
        let profile_path = write_paymaster_profile(&profile)?;

        let mut command = Command::new(bin);
        command
            .env("PAYMASTER_PROFILE", &profile_path)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);

        info!(target: "sidecar", profile = %profile_path.display(), "paymaster profile generated");

        let process = command.spawn().context("failed to spawn paymaster sidecar")?;

        let url = Url::parse(&format!("http://127.0.0.1:{}", config.port)).expect("valid url");
        wait_for_paymaster_ready(&url, Some(&config.api_key), BOOTSTRAP_TIMEOUT).await?;

        Ok(PaymasterSidecarProcess { process, config })
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

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
