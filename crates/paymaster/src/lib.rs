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
use std::{env, fs, io};

use katana_primitives::chain::{ChainId, NamedChainId};
use katana_primitives::class::ComputeClassHashError;
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
use thiserror::Error;
use tokio::process::{Child, Command};
use tokio::time::sleep;
use tracing::{debug, info, warn};
use url::Url;

const FORWARDER_SALT: u64 = 0x12345;
const BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_AVNU_PRICE_MAINNET_ENDPOINT: &str = "https://starknet.impulse.avnu.fi/v3/";

// ============================================================================
// Error Types
// ============================================================================

/// Result type for paymaster operations.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Errors that can occur during paymaster operations.
#[derive(Debug, Error)]
pub enum Error {
    /// A required configuration field is missing.
    #[error("missing required field: {0}")]
    MissingField(&'static str),

    /// An account does not exist on chain.
    #[error("{kind} account {address} does not exist on chain")]
    AccountNotDeployed { kind: &'static str, address: ContractAddress },

    /// Forwarder address not set before starting.
    #[error("forwarder_address not set - call bootstrap() or forwarder()")]
    ForwarderNotSet,

    /// Chain ID not set before starting.
    #[error("chain_id not set - call bootstrap() or chain_id()")]
    ChainIdNotSet,

    /// Failed to get chain ID from the provider.
    #[error("failed to get chain ID from node")]
    ChainId(#[source] Box<ProviderError>),

    /// Failed to check if a contract is deployed.
    #[error("failed to check contract deployment at {0}")]
    ContractCheck(ContractAddress, #[source] Box<ProviderError>),

    /// Failed to deploy the forwarder contract.
    #[error("failed to deploy forwarder: {0}")]
    ForwarderDeploy(String),

    /// Failed to whitelist the relayer.
    #[error("failed to whitelist relayer: {0}")]
    WhitelistRelayer(String),

    /// Contract was not deployed within the timeout period.
    #[error("contract {0} not deployed before timeout")]
    ContractDeployTimeout(ContractAddress),

    /// Sidecar binary not found.
    #[error("sidecar binary not found at {0}")]
    BinaryNotFound(PathBuf),

    /// Sidecar binary not found in PATH.
    #[error("sidecar binary '{0}' not found in PATH")]
    BinaryNotInPath(PathBuf),

    /// PATH environment variable is not set.
    #[error("PATH environment variable is not set")]
    PathNotSet,

    /// Failed to spawn the sidecar process.
    #[error("failed to spawn paymaster sidecar")]
    Spawn(#[source] io::Error),

    /// Failed to compute the forwarder class hash.
    #[error("failed to compute forwarder class hash")]
    ClassHash(#[source] ComputeClassHashError),

    /// Failed to parse the forwarder contract class.
    #[error("failed to parse forwarder contract class")]
    ClassParse(#[source] serde_json::Error),

    /// Failed to serialize the paymaster profile.
    #[error("failed to serialize paymaster profile")]
    ProfileSerialize(#[source] serde_json::Error),

    /// Failed to write the paymaster profile to disk.
    #[error("failed to write paymaster profile")]
    ProfileWrite(#[source] io::Error),

    /// Paymaster did not become ready within the timeout period.
    #[error("paymaster did not become ready before timeout")]
    SidecarTimeout,
}

#[derive(Debug)]
pub struct PaymasterSidecarProcess {
    process: Child,
    profile: PaymasterProfile,
}

impl PaymasterSidecarProcess {
    pub fn process(&mut self) -> &mut Child {
        &mut self.process
    }

    pub fn profile(&self) -> &PaymasterProfile {
        &self.profile
    }

    /// Gracefully shutdown the sidecar process.
    pub async fn shutdown(&mut self) -> std::io::Result<()> {
        self.process.kill().await
    }
}

// ============================================================================
// PaymasterConfig and PaymasterConfigBuilder
// ============================================================================

/// Validated paymaster configuration.
///
/// This struct contains all the configuration needed to start a paymaster sidecar.
/// Use [`PaymasterConfigBuilder`] to construct this.
#[derive(Debug, Clone)]
pub struct PaymasterConfig {
    /// RPC URL of the katana node.
    pub rpc_url: Url,
    /// Port for the paymaster service.
    pub port: u16,
    /// API key for the paymaster service.
    pub api_key: String,
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
    /// Path to the paymaster-service binary, or None to look up in PATH.
    pub program_path: Option<PathBuf>,
    /// Price API key (for AVNU price feed).
    pub price_api_key: Option<String>,
}

/// Builder for [`PaymasterConfig`] - ensures all required fields are set.
///
/// The builder validates all required arguments at build time and fails if any are missing.
/// It also validates that all account addresses exist on-chain via RPC.
///
/// # Example
///
/// ```ignore
/// let config = PaymasterConfigBuilder::new()
///     .rpc_url(rpc_url)
///     .port(3030)
///     .api_key("paymaster_key".to_string())
///     .relayer(relayer_addr, relayer_key)
///     .gas_tank(gas_tank_addr, gas_tank_key)
///     .estimate_account(estimate_addr, estimate_key)
///     .tokens(eth_addr, strk_addr)
///     .build()
///     .await?;
/// ```
#[derive(Debug, Default)]
pub struct PaymasterConfigBuilder {
    // Required fields
    rpc_url: Option<Url>,
    port: Option<u16>,
    api_key: Option<String>,
    relayer_address: Option<ContractAddress>,
    relayer_private_key: Option<Felt>,
    gas_tank_address: Option<ContractAddress>,
    gas_tank_private_key: Option<Felt>,
    estimate_account_address: Option<ContractAddress>,
    estimate_account_private_key: Option<Felt>,
    eth_token_address: Option<ContractAddress>,
    strk_token_address: Option<ContractAddress>,

    // Optional fields
    program_path: Option<PathBuf>,
    price_api_key: Option<String>,
}

impl PaymasterConfigBuilder {
    /// Create a new builder with no fields set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the RPC URL of the katana node.
    pub fn rpc_url(mut self, url: Url) -> Self {
        self.rpc_url = Some(url);
        self
    }

    /// Set the port for the paymaster service.
    pub fn port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }

    /// Set the API key for the paymaster service.
    pub fn api_key(mut self, key: String) -> Self {
        self.api_key = Some(key);
        self
    }

    /// Set relayer account credentials.
    pub fn relayer(mut self, address: ContractAddress, private_key: Felt) -> Self {
        self.relayer_address = Some(address);
        self.relayer_private_key = Some(private_key);
        self
    }

    /// Set gas tank account credentials.
    pub fn gas_tank(mut self, address: ContractAddress, private_key: Felt) -> Self {
        self.gas_tank_address = Some(address);
        self.gas_tank_private_key = Some(private_key);
        self
    }

    /// Set estimation account credentials.
    pub fn estimate_account(mut self, address: ContractAddress, private_key: Felt) -> Self {
        self.estimate_account_address = Some(address);
        self.estimate_account_private_key = Some(private_key);
        self
    }

    /// Set token addresses.
    pub fn tokens(mut self, eth: ContractAddress, strk: ContractAddress) -> Self {
        self.eth_token_address = Some(eth);
        self.strk_token_address = Some(strk);
        self
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

    /// Build the [`PaymasterConfig`], validating all required fields.
    ///
    /// Also validates that all account addresses exist on-chain via RPC.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Any required field is missing
    /// - Any account address does not exist on-chain
    pub async fn build(self) -> Result<PaymasterConfig> {
        // Validate required fields
        let rpc_url = self.rpc_url.ok_or(Error::MissingField("rpc_url"))?;
        let port = self.port.ok_or(Error::MissingField("port"))?;
        let api_key = self.api_key.ok_or(Error::MissingField("api_key"))?;
        let relayer_address = self.relayer_address.ok_or(Error::MissingField("relayer_address"))?;
        let relayer_private_key =
            self.relayer_private_key.ok_or(Error::MissingField("relayer_private_key"))?;
        let gas_tank_address =
            self.gas_tank_address.ok_or(Error::MissingField("gas_tank_address"))?;
        let gas_tank_private_key =
            self.gas_tank_private_key.ok_or(Error::MissingField("gas_tank_private_key"))?;
        let estimate_account_address =
            self.estimate_account_address.ok_or(Error::MissingField("estimate_account_address"))?;
        let estimate_account_private_key = self
            .estimate_account_private_key
            .ok_or(Error::MissingField("estimate_account_private_key"))?;
        let eth_token_address =
            self.eth_token_address.ok_or(Error::MissingField("eth_token_address"))?;
        let strk_token_address =
            self.strk_token_address.ok_or(Error::MissingField("strk_token_address"))?;

        // Validate accounts exist on-chain
        let provider = JsonRpcClient::new(HttpTransport::new(rpc_url.clone()));

        if !is_deployed(&provider, relayer_address).await? {
            return Err(Error::AccountNotDeployed { kind: "relayer", address: relayer_address });
        }

        if !is_deployed(&provider, gas_tank_address).await? {
            return Err(Error::AccountNotDeployed { kind: "gas tank", address: gas_tank_address });
        }

        if !is_deployed(&provider, estimate_account_address).await? {
            return Err(Error::AccountNotDeployed {
                kind: "estimate",
                address: estimate_account_address,
            });
        }

        Ok(PaymasterConfig {
            rpc_url,
            port,
            api_key,
            relayer_address,
            relayer_private_key,
            gas_tank_address,
            gas_tank_private_key,
            estimate_account_address,
            estimate_account_private_key,
            eth_token_address,
            strk_token_address,
            program_path: self.program_path,
            price_api_key: self.price_api_key,
        })
    }

    /// Build the [`PaymasterConfig`] without on-chain validation.
    ///
    /// This method validates that all required fields are set but does NOT check
    /// if accounts exist on-chain. Use this for bootstrap scenarios where accounts
    /// are known to exist from genesis.
    ///
    /// # Errors
    ///
    /// Returns an error if any required field is missing.
    pub fn build_unchecked(self) -> Result<PaymasterConfig> {
        // Validate required fields
        let rpc_url = self.rpc_url.ok_or(Error::MissingField("rpc_url"))?;
        let port = self.port.ok_or(Error::MissingField("port"))?;
        let api_key = self.api_key.ok_or(Error::MissingField("api_key"))?;
        let relayer_address = self.relayer_address.ok_or(Error::MissingField("relayer_address"))?;
        let relayer_private_key =
            self.relayer_private_key.ok_or(Error::MissingField("relayer_private_key"))?;
        let gas_tank_address =
            self.gas_tank_address.ok_or(Error::MissingField("gas_tank_address"))?;
        let gas_tank_private_key =
            self.gas_tank_private_key.ok_or(Error::MissingField("gas_tank_private_key"))?;
        let estimate_account_address =
            self.estimate_account_address.ok_or(Error::MissingField("estimate_account_address"))?;
        let estimate_account_private_key = self
            .estimate_account_private_key
            .ok_or(Error::MissingField("estimate_account_private_key"))?;
        let eth_token_address =
            self.eth_token_address.ok_or(Error::MissingField("eth_token_address"))?;
        let strk_token_address =
            self.strk_token_address.ok_or(Error::MissingField("strk_token_address"))?;

        Ok(PaymasterConfig {
            rpc_url,
            port,
            api_key,
            relayer_address,
            relayer_private_key,
            gas_tank_address,
            gas_tank_private_key,
            estimate_account_address,
            estimate_account_private_key,
            eth_token_address,
            strk_token_address,
            program_path: self.program_path,
            price_api_key: self.price_api_key,
        })
    }
}

// ============================================================================
// PaymasterSidecar
// ============================================================================

/// Paymaster sidecar - handles bootstrapping and starting the sidecar process.
///
/// This struct accepts a validated [`PaymasterConfig`] and provides methods to:
/// - Bootstrap the paymaster (deploy forwarder contract, whitelist relayer)
/// - Start the sidecar process
///
/// # Example
///
/// ```ignore
/// // Build and validate config
/// let config = PaymasterConfigBuilder::new()
///     .rpc_url(rpc_url)
///     .port(3030)
///     .api_key("paymaster_key".to_string())
///     .relayer(relayer_addr, relayer_key)
///     .gas_tank(gas_tank_addr, gas_tank_key)
///     .estimate_account(estimate_addr, estimate_key)
///     .tokens(eth_addr, strk_addr)
///     .build()
///     .await?;
///
/// // Create sidecar and bootstrap
/// let mut sidecar = PaymasterSidecar::new(config);
/// sidecar.bootstrap().await?;
/// let process = sidecar.start().await?;
///
/// // Or skip bootstrap if forwarder/chain_id are known
/// let process = PaymasterSidecar::new(config)
///     .forwarder(known_forwarder)
///     .chain_id(ChainId::SEPOLIA)
///     .start()
///     .await?;
/// ```
#[derive(Debug, Clone)]
pub struct PaymasterSidecar {
    config: PaymasterConfig,

    // Bootstrap-derived (can be set directly or via bootstrap)
    forwarder_address: Option<ContractAddress>,
    chain_id: Option<ChainId>,
}

impl PaymasterSidecar {
    /// Create a new sidecar from a validated config.
    pub fn new(config: PaymasterConfig) -> Self {
        Self { config, forwarder_address: None, chain_id: None }
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

    /// Get the chain ID if set.
    pub fn get_chain_id(&self) -> Option<ChainId> {
        self.chain_id
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
        let provider =
            Arc::new(JsonRpcClient::new(HttpTransport::new(self.config.rpc_url.clone())));

        // Get chain ID if not already set
        let chain_id_felt = if let Some(chain_id) = &self.chain_id {
            chain_id.id()
        } else {
            let chain_id_felt =
                provider.chain_id().await.map_err(|e| Error::ChainId(Box::new(e)))?;
            self.chain_id = Some(ChainId::Id(chain_id_felt));
            chain_id_felt
        };

        let forwarder_class_hash = avnu_forwarder_class_hash()?;
        // When using UDC with unique=0 (non-unique deployment), the deployer_address
        // used in address computation is 0, not the actual deployer or UDC address.
        let forwarder_address = get_contract_address(
            Felt::from(FORWARDER_SALT),
            forwarder_class_hash,
            &[self.config.relayer_address.into(), self.config.gas_tank_address.into()],
            Felt::ZERO,
        )
        .into();

        // Create the relayer account for transactions
        let secret_key = SigningKey::from_secret_scalar(self.config.relayer_private_key);
        let account = SingleOwnerAccount::new(
            provider.clone(),
            LocalWallet::from(secret_key),
            self.config.relayer_address.into(),
            chain_id_felt,
            ExecutionEncoding::New,
        );

        // Deploy forwarder if not already deployed
        if !is_deployed(&provider, forwarder_address).await? {
            #[allow(deprecated)]
            let factory = ContractFactory::new(forwarder_class_hash, &account);

            factory
                .deploy_v3(
                    vec![self.config.relayer_address.into(), self.config.gas_tank_address.into()],
                    Felt::from(FORWARDER_SALT),
                    false,
                )
                .send()
                .await
                .map_err(|e| Error::ForwarderDeploy(e.to_string()))?;

            wait_for_contract(&provider, forwarder_address, BOOTSTRAP_TIMEOUT).await?;
        }

        // Whitelist the relayer
        let whitelist_call = Call {
            to: forwarder_address.into(),
            selector: selector!("set_whitelisted_address"),
            calldata: vec![self.config.relayer_address.into(), Felt::ONE],
        };

        account
            .execute_v3(vec![whitelist_call])
            .send()
            .await
            .map_err(|e| Error::WhitelistRelayer(e.to_string()))?;

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
        // Build profile and spawn process
        let bin =
            self.config.program_path.clone().unwrap_or_else(|| PathBuf::from("paymaster-service"));
        let bin = resolve_executable(&bin)?;
        let profile = self.build_paymaster_profile()?;
        let profile_path = write_paymaster_profile(&profile)?;

        let mut command = Command::new(bin);
        command
            .env("PAYMASTER_PROFILE", &profile_path)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);

        info!(target: "sidecar", profile = %profile_path.display(), "paymaster profile generated");

        let process = command.spawn().map_err(Error::Spawn)?;

        let url = Url::parse(&format!("http://127.0.0.1:{}", self.config.port)).expect("valid url");
        wait_for_paymaster_ready(&url, Some(&self.config.api_key), BOOTSTRAP_TIMEOUT).await?;

        Ok(PaymasterSidecarProcess { process, profile })
    }

    fn build_paymaster_profile(&self) -> Result<PaymasterProfile> {
        let forwarder_address = self.forwarder_address.ok_or(Error::ForwarderNotSet)?;
        let chain_id = self.chain_id.ok_or(Error::ChainIdNotSet)?;

        let chain_id_str = paymaster_chain_id(chain_id);
        let price_api_key = self.config.price_api_key.clone().unwrap_or_default();

        Ok(PaymasterProfile {
            verbosity: "info".to_string(),
            prometheus: None,
            rpc: PaymasterRpcProfile { port: self.config.port },
            forwarder: forwarder_address,
            supported_tokens: vec![self.config.eth_token_address, self.config.strk_token_address],
            max_fee_multiplier: 3.0,
            provider_fee_overhead: 0.1,
            estimate_account: PaymasterAccountProfile {
                address: self.config.estimate_account_address,
                private_key: self.config.estimate_account_private_key,
            },
            gas_tank: PaymasterAccountProfile {
                address: self.config.gas_tank_address,
                private_key: self.config.gas_tank_private_key,
            },
            relayers: PaymasterRelayersProfile {
                private_key: self.config.relayer_private_key,
                addresses: vec![self.config.relayer_address],
                min_relayer_balance: Felt::ZERO,
                lock: PaymasterLockProfile { mode: "seggregated".to_string(), retry_timeout: 5 },
            },
            starknet: PaymasterStarknetProfile {
                chain_id: chain_id_str,
                endpoint: self.config.rpc_url.clone(),
                timeout: 30,
                fallbacks: Vec::new(),
            },
            price: PaymasterPriceProfile {
                provider: "avnu".to_string(),
                endpoint: Url::parse(DEFAULT_AVNU_PRICE_MAINNET_ENDPOINT).expect("valid url"),
                api_key: price_api_key,
            },
            sponsoring: PaymasterSponsoringProfile {
                mode: "self".to_string(),
                api_key: self.config.api_key.clone(),
                sponsor_metadata: Vec::new(),
            },
        })
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
        Err(e) => Err(Error::ContractCheck(address, Box::new(e))),
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
            return Err(Error::ContractDeployTimeout(address));
        }

        sleep(Duration::from_millis(200)).await;
    }
}

fn avnu_forwarder_class_hash() -> Result<Felt> {
    let class = katana_primitives::utils::class::parse_sierra_class(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../controller/classes/avnu_Forwarder.contract_class.json"
    )))
    .map_err(Error::ClassParse)?;
    class.class_hash().map_err(Error::ClassHash)
}

fn resolve_executable(path: &Path) -> Result<PathBuf> {
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

// ============================================================================
// Paymaster Profile
// ============================================================================

#[derive(Debug, Serialize)]
pub struct PaymasterProfile {
    pub verbosity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prometheus: Option<PaymasterPrometheusProfile>,
    pub rpc: PaymasterRpcProfile,
    #[serde(serialize_with = "ser::contract_address")]
    pub forwarder: ContractAddress,
    #[serde(serialize_with = "ser::contract_address_vec")]
    pub supported_tokens: Vec<ContractAddress>,
    pub max_fee_multiplier: f32,
    pub provider_fee_overhead: f32,
    pub estimate_account: PaymasterAccountProfile,
    pub gas_tank: PaymasterAccountProfile,
    pub relayers: PaymasterRelayersProfile,
    pub starknet: PaymasterStarknetProfile,
    pub price: PaymasterPriceProfile,
    pub sponsoring: PaymasterSponsoringProfile,
}

#[derive(Debug, Serialize)]
pub struct PaymasterPrometheusProfile {
    #[serde(serialize_with = "ser::url")]
    pub endpoint: Url,
}

#[derive(Debug, Serialize)]
pub struct PaymasterRpcProfile {
    pub port: u16,
}

#[derive(Debug, Serialize)]
pub struct PaymasterAccountProfile {
    #[serde(serialize_with = "ser::contract_address")]
    pub address: ContractAddress,
    #[serde(serialize_with = "ser::felt")]
    pub private_key: Felt,
}

#[derive(Debug, Serialize)]
pub struct PaymasterRelayersProfile {
    #[serde(serialize_with = "ser::felt")]
    pub private_key: Felt,
    #[serde(serialize_with = "ser::contract_address_vec")]
    pub addresses: Vec<ContractAddress>,
    #[serde(serialize_with = "ser::felt")]
    pub min_relayer_balance: Felt,
    pub lock: PaymasterLockProfile,
}

#[derive(Debug, Serialize)]
pub struct PaymasterLockProfile {
    pub mode: String,
    pub retry_timeout: u64,
}

#[derive(Debug, Serialize)]
pub struct PaymasterStarknetProfile {
    pub chain_id: String,
    #[serde(serialize_with = "ser::url")]
    pub endpoint: Url,
    pub timeout: u64,
    #[serde(serialize_with = "ser::url_vec")]
    pub fallbacks: Vec<Url>,
}

#[derive(Debug, Serialize)]
pub struct PaymasterPriceProfile {
    pub provider: String,
    #[serde(serialize_with = "ser::url")]
    pub endpoint: Url,
    pub api_key: String,
}

#[derive(Debug, Serialize)]
pub struct PaymasterSponsoringProfile {
    pub mode: String,
    pub api_key: String,
    #[serde(serialize_with = "ser::felt_vec")]
    pub sponsor_metadata: Vec<Felt>,
}

/// Custom serializers for paymaster profile types.
mod ser {
    use katana_primitives::{ContractAddress, Felt};
    use serde::Serializer;
    use url::Url;

    /// Serialize a Felt as a hex string with 0x prefix.
    pub fn felt<S: Serializer>(value: &Felt, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format!("{value:#x}"))
    }

    /// Serialize a Vec<Felt> as a vec of hex strings.
    pub fn felt_vec<S: Serializer>(values: &[Felt], serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let mut seq = serializer.serialize_seq(Some(values.len()))?;
        for value in values {
            seq.serialize_element(&format!("{value:#x}"))?;
        }
        seq.end()
    }

    /// Serialize a ContractAddress as a hex string with 0x prefix.
    pub fn contract_address<S: Serializer>(
        value: &ContractAddress,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let felt: Felt = (*value).into();
        serializer.serialize_str(&format!("{felt:#x}"))
    }

    /// Serialize a Vec<ContractAddress> as a vec of hex strings.
    pub fn contract_address_vec<S: Serializer>(
        values: &[ContractAddress],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let mut seq = serializer.serialize_seq(Some(values.len()))?;
        for value in values {
            let felt: Felt = (*value).into();
            seq.serialize_element(&format!("{felt:#x}"))?;
        }
        seq.end()
    }

    /// Serialize a Url as a string.
    pub fn url<S: Serializer>(value: &Url, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(value.as_str())
    }

    /// Serialize a Vec<Url> as a vec of strings.
    pub fn url_vec<S: Serializer>(values: &[Url], serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let mut seq = serializer.serialize_seq(Some(values.len()))?;
        for value in values {
            seq.serialize_element(value.as_str())?;
        }
        seq.end()
    }
}

fn write_paymaster_profile(profile: &PaymasterProfile) -> Result<PathBuf> {
    let payload = serde_json::to_string_pretty(profile).map_err(Error::ProfileSerialize)?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_millis();
    let pid = std::process::id();

    let mut path = env::temp_dir();
    path.push(format!("katana-paymaster-profile-{timestamp}-{pid}.json"));
    fs::write(&path, payload).map_err(Error::ProfileWrite)?;
    Ok(path)
}

fn paymaster_chain_id(chain_id: ChainId) -> String {
    match chain_id {
        ChainId::Named(NamedChainId::Mainnet) => "mainnet".to_string(),
        _ => "sepolia".to_string(),
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
            return Err(Error::SidecarTimeout);
        }

        sleep(Duration::from_millis(200)).await;
    }
}

/// Format a Felt as a hex string with 0x prefix.
pub fn format_felt(value: Felt) -> String {
    format!("{value:#x}")
}
