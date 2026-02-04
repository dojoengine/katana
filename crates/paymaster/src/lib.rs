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

const FORWARDER_SALT: u64 = 0x12345;
const BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_AVNU_PRICE_MAINNET_ENDPOINT: &str = "https://starknet.impulse.avnu.fi/v3/";

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
    pub async fn shutdown(&mut self) -> Result<()> {
        self.process.kill().await?;
        Ok(())
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
        let rpc_url = self.rpc_url.ok_or_else(|| anyhow!("missing required field: rpc_url"))?;
        let port = self.port.ok_or_else(|| anyhow!("missing required field: port"))?;
        let api_key = self.api_key.ok_or_else(|| anyhow!("missing required field: api_key"))?;
        let relayer_address = self
            .relayer_address
            .ok_or_else(|| anyhow!("missing required field: relayer_address"))?;
        let relayer_private_key = self
            .relayer_private_key
            .ok_or_else(|| anyhow!("missing required field: relayer_private_key"))?;
        let gas_tank_address = self
            .gas_tank_address
            .ok_or_else(|| anyhow!("missing required field: gas_tank_address"))?;
        let gas_tank_private_key = self
            .gas_tank_private_key
            .ok_or_else(|| anyhow!("missing required field: gas_tank_private_key"))?;
        let estimate_account_address = self
            .estimate_account_address
            .ok_or_else(|| anyhow!("missing required field: estimate_account_address"))?;
        let estimate_account_private_key = self
            .estimate_account_private_key
            .ok_or_else(|| anyhow!("missing required field: estimate_account_private_key"))?;
        let eth_token_address = self
            .eth_token_address
            .ok_or_else(|| anyhow!("missing required field: eth_token_address"))?;
        let strk_token_address = self
            .strk_token_address
            .ok_or_else(|| anyhow!("missing required field: strk_token_address"))?;

        // Validate accounts exist on-chain
        let provider = JsonRpcClient::new(HttpTransport::new(rpc_url.clone()));

        if !is_deployed(&provider, relayer_address).await? {
            return Err(anyhow!("relayer account {relayer_address} does not exist on chain"));
        }

        if !is_deployed(&provider, gas_tank_address).await? {
            return Err(anyhow!("gas tank account {gas_tank_address} does not exist on chain"));
        }

        if !is_deployed(&provider, estimate_account_address).await? {
            return Err(anyhow!(
                "estimate account {estimate_account_address} does not exist on chain"
            ));
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
        let rpc_url = self.rpc_url.ok_or_else(|| anyhow!("missing required field: rpc_url"))?;
        let port = self.port.ok_or_else(|| anyhow!("missing required field: port"))?;
        let api_key = self.api_key.ok_or_else(|| anyhow!("missing required field: api_key"))?;
        let relayer_address = self
            .relayer_address
            .ok_or_else(|| anyhow!("missing required field: relayer_address"))?;
        let relayer_private_key = self
            .relayer_private_key
            .ok_or_else(|| anyhow!("missing required field: relayer_private_key"))?;
        let gas_tank_address = self
            .gas_tank_address
            .ok_or_else(|| anyhow!("missing required field: gas_tank_address"))?;
        let gas_tank_private_key = self
            .gas_tank_private_key
            .ok_or_else(|| anyhow!("missing required field: gas_tank_private_key"))?;
        let estimate_account_address = self
            .estimate_account_address
            .ok_or_else(|| anyhow!("missing required field: estimate_account_address"))?;
        let estimate_account_private_key = self
            .estimate_account_private_key
            .ok_or_else(|| anyhow!("missing required field: estimate_account_private_key"))?;
        let eth_token_address = self
            .eth_token_address
            .ok_or_else(|| anyhow!("missing required field: eth_token_address"))?;
        let strk_token_address = self
            .strk_token_address
            .ok_or_else(|| anyhow!("missing required field: strk_token_address"))?;

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
                .map_err(|e| anyhow!("failed to deploy forwarder: {e}"))?;

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

        // Build profile and spawn process
        let bin =
            self.config.program_path.clone().unwrap_or_else(|| PathBuf::from("paymaster-service"));
        let bin = resolve_executable(&bin)?;
        let profile = build_paymaster_profile(&self.config, forwarder_address, chain_id)?;
        let profile_path = write_paymaster_profile(&profile)?;

        let mut command = Command::new(bin);
        command
            .env("PAYMASTER_PROFILE", &profile_path)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);

        info!(target: "sidecar", profile = %profile_path.display(), "paymaster profile generated");

        let process = command.spawn().context("failed to spawn paymaster sidecar")?;

        let url = Url::parse(&format!("http://127.0.0.1:{}", self.config.port)).expect("valid url");
        wait_for_paymaster_ready(&url, Some(&self.config.api_key), BOOTSTRAP_TIMEOUT).await?;

        Ok(PaymasterSidecarProcess { process, profile })
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

fn build_paymaster_profile(
    config: &PaymasterConfig,
    forwarder_address: ContractAddress,
    chain_id: ChainId,
) -> Result<PaymasterProfile> {
    let chain_id_str = paymaster_chain_id(chain_id)?;
    let price_api_key = config.price_api_key.clone().unwrap_or_default();

    Ok(PaymasterProfile {
        verbosity: "info".to_string(),
        prometheus: None,
        rpc: PaymasterRpcProfile { port: config.port },
        forwarder: forwarder_address,
        supported_tokens: vec![config.eth_token_address, config.strk_token_address],
        max_fee_multiplier: 3.0,
        provider_fee_overhead: 0.1,
        estimate_account: PaymasterAccountProfile {
            address: config.estimate_account_address,
            private_key: config.estimate_account_private_key,
        },
        gas_tank: PaymasterAccountProfile {
            address: config.gas_tank_address,
            private_key: config.gas_tank_private_key,
        },
        relayers: PaymasterRelayersProfile {
            private_key: config.relayer_private_key,
            addresses: vec![config.relayer_address],
            min_relayer_balance: Felt::ZERO,
            lock: PaymasterLockProfile { mode: "seggregated".to_string(), retry_timeout: 5 },
        },
        starknet: PaymasterStarknetProfile {
            chain_id: chain_id_str,
            endpoint: config.rpc_url.clone(),
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
