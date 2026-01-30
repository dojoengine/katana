//! Paymaster sidecar bootstrap and process management.
//!
//! This crate handles:
//! - Bootstrapping the paymaster service (deploying forwarder contract via RPC)
//! - Spawning and managing the paymaster sidecar process
//! - Generating paymaster configuration profiles
//!
//! This crate is self-contained and uses the Starknet RPC client to interact with
//! the katana node for bootstrap operations.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::{env, fs};

use anyhow::{anyhow, Context, Result};
use katana_primitives::block::BlockIdOrTag;
use katana_primitives::chain::{ChainId, NamedChainId};
use katana_primitives::da::DataAvailabilityMode;
use katana_primitives::fee::{AllResourceBoundsMapping, ResourceBoundsMapping, Tip};
use katana_primitives::utils::get_contract_address;
use katana_primitives::{ContractAddress, Felt};
use katana_rpc_client::starknet::Client;
use katana_rpc_types::broadcasted::{BroadcastedInvokeTx, BroadcastedTxWithChainId};
use katana_rpc_types::FunctionCall;
use serde::Serialize;
use starknet::macros::selector;
use starknet::signers::{LocalWallet, Signer, SigningKey};
use tokio::process::{Child, Command};
use tokio::time::sleep;
use tracing::{debug, info, warn};
use url::Url;

// ============================================================================
// Constants
// ============================================================================

const FORWARDER_SALT: u64 = 0x12345;
const BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_AVNU_PRICE_SEPOLIA_ENDPOINT: &str = "https://sepolia.api.avnu.fi";
const DEFAULT_AVNU_PRICE_MAINNET_ENDPOINT: &str = "https://starknet.api.avnu.fi";

/// The default universal deployer contract address.
pub const DEFAULT_UDC_ADDRESS: ContractAddress = ContractAddress(katana_primitives::felt!(
    "0x041a78e741e5af2fec34b695679bc6891742439f7afb8484ecd7766661ad02bf"
));

/// The default ETH fee token contract address.
pub const DEFAULT_ETH_FEE_TOKEN_ADDRESS: ContractAddress = ContractAddress(
    katana_primitives::felt!("0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7"),
);

/// The default STRK fee token contract address.
pub const DEFAULT_STRK_FEE_TOKEN_ADDRESS: ContractAddress = ContractAddress(
    katana_primitives::felt!("0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d"),
);

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
    pub bin: Option<PathBuf>,
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
    let client = Client::new(config.rpc_url.clone());

    // Get chain ID from the node
    let chain_id_felt = client.chain_id().await.context("failed to get chain ID from node")?;
    let chain_id = ChainId::Id(chain_id_felt);

    let forwarder_class_hash = avnu_forwarder_class_hash()?;
    let forwarder_address = get_contract_address(
        Felt::from(FORWARDER_SALT),
        forwarder_class_hash,
        &[config.relayer_address.into(), config.gas_tank_address.into()],
        DEFAULT_UDC_ADDRESS.into(),
    )
    .into();

    // Deploy forwarder if not already deployed
    ensure_deployed(
        &client,
        chain_id,
        DeploymentRequest {
            sender_address: config.relayer_address,
            sender_private_key: config.relayer_private_key,
            target_address: forwarder_address,
            class_hash: forwarder_class_hash,
            constructor_calldata: vec![
                config.relayer_address.into(),
                config.gas_tank_address.into(),
            ],
            salt: Felt::from(FORWARDER_SALT),
        },
    )
    .await?;

    // Whitelist the relayer
    let whitelist_call = FunctionCall {
        contract_address: forwarder_address,
        entry_point_selector: selector!("set_whitelisted_address"),
        calldata: vec![config.relayer_address.into(), Felt::ONE],
    };

    submit_invoke(
        &client,
        chain_id,
        config.relayer_address,
        config.relayer_private_key,
        vec![whitelist_call],
    )
    .await?;

    Ok(PaymasterBootstrapResult { forwarder_address, chain_id })
}

struct DeploymentRequest {
    sender_address: ContractAddress,
    sender_private_key: Felt,
    target_address: ContractAddress,
    class_hash: Felt,
    constructor_calldata: Vec<Felt>,
    salt: Felt,
}

async fn ensure_deployed(
    client: &Client,
    chain_id: ChainId,
    request: DeploymentRequest,
) -> Result<()> {
    let DeploymentRequest {
        sender_address,
        sender_private_key,
        target_address,
        class_hash,
        constructor_calldata,
        salt,
    } = request;

    if is_deployed(client, target_address).await? {
        return Ok(());
    }

    let deploy_call = FunctionCall {
        contract_address: DEFAULT_UDC_ADDRESS,
        entry_point_selector: selector!("deployContract"),
        calldata: udc_calldata(class_hash, salt, constructor_calldata),
    };

    submit_invoke(client, chain_id, sender_address, sender_private_key, vec![deploy_call]).await?;

    wait_for_contract(client, target_address, BOOTSTRAP_TIMEOUT).await?;
    Ok(())
}

async fn submit_invoke(
    client: &Client,
    chain_id: ChainId,
    sender_address: ContractAddress,
    sender_private_key: Felt,
    calls: Vec<FunctionCall>,
) -> Result<()> {
    let nonce = client
        .get_nonce(BlockIdOrTag::PreConfirmed, sender_address)
        .await
        .context("failed to get nonce")?;

    let tx = build_and_sign_invoke_tx(chain_id, sender_address, sender_private_key, nonce, calls)?;

    client.add_invoke_transaction(tx).await.context("failed to submit invoke transaction")?;

    Ok(())
}

fn build_and_sign_invoke_tx(
    chain_id: ChainId,
    sender_address: ContractAddress,
    sender_private_key: Felt,
    nonce: Felt,
    calls: Vec<FunctionCall>,
) -> Result<BroadcastedInvokeTx> {
    // Build an unsigned transaction to compute the hash
    let unsigned_tx = BroadcastedInvokeTx {
        sender_address,
        calldata: encode_calls(calls),
        signature: vec![],
        nonce,
        paymaster_data: vec![],
        tip: Tip::new(0),
        account_deployment_data: vec![],
        resource_bounds: ResourceBoundsMapping::All(AllResourceBoundsMapping::default()),
        fee_data_availability_mode: DataAvailabilityMode::L1,
        nonce_data_availability_mode: DataAvailabilityMode::L1,
        is_query: false,
    };

    // Compute the transaction hash using BroadcastedTxWithChainId
    let tx_with_chain = BroadcastedTxWithChainId {
        tx: katana_rpc_types::broadcasted::BroadcastedTx::Invoke(unsigned_tx.clone()),
        chain: chain_id,
    };
    let tx_hash = tx_with_chain.calculate_hash();

    // Sign the transaction hash
    let signer = LocalWallet::from(SigningKey::from_secret_scalar(sender_private_key));
    let signature = futures::executor::block_on(signer.sign_hash(&tx_hash))
        .map_err(|e| anyhow!("failed to sign transaction: {e}"))?;

    // Return signed transaction
    Ok(BroadcastedInvokeTx { signature: vec![signature.r, signature.s], ..unsigned_tx })
}

fn encode_calls(calls: Vec<FunctionCall>) -> Vec<Felt> {
    let mut execute_calldata: Vec<Felt> = vec![calls.len().into()];
    for call in calls {
        execute_calldata.push(call.contract_address.into());
        execute_calldata.push(call.entry_point_selector);

        execute_calldata.push(call.calldata.len().into());
        execute_calldata.extend_from_slice(&call.calldata);
    }

    execute_calldata
}

fn udc_calldata(class_hash: Felt, salt: Felt, constructor_calldata: Vec<Felt>) -> Vec<Felt> {
    let mut calldata = Vec::with_capacity(4 + constructor_calldata.len());
    calldata.push(class_hash);
    calldata.push(salt);
    calldata.push(Felt::ZERO);
    calldata.push(Felt::from(constructor_calldata.len()));
    calldata.extend(constructor_calldata);
    calldata
}

async fn is_deployed(client: &Client, address: ContractAddress) -> Result<bool> {
    match client.get_class_hash_at(BlockIdOrTag::PreConfirmed, address).await {
        Ok(_) => Ok(true),
        Err(katana_rpc_client::starknet::Error::Starknet(
            katana_rpc_client::starknet::StarknetApiError::ContractNotFound,
        )) => Ok(false),
        Err(e) => Err(anyhow!("failed to check contract deployment: {e}")),
    }
}

async fn wait_for_contract(
    client: &Client,
    address: ContractAddress,
    timeout: Duration,
) -> Result<()> {
    let start = Instant::now();
    loop {
        if is_deployed(client, address).await? {
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
    let bin = config.bin.clone().unwrap_or_else(|| PathBuf::from("paymaster-service"));
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
    let price_endpoint = paymaster_price_endpoint(config.chain_id)?;
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
        ChainId::Named(NamedChainId::Sepolia) => Ok("sepolia".to_string()),
        ChainId::Named(NamedChainId::Mainnet) => Ok("mainnet".to_string()),
        ChainId::Named(other) => Err(anyhow!(
            "paymaster sidecar only supports SN_MAIN or SN_SEPOLIA chain ids, got {other}"
        )),
        ChainId::Id(id) => {
            // Check if the id matches known chain IDs
            if id == ChainId::SEPOLIA.id() {
                Ok("sepolia".to_string())
            } else if id == ChainId::MAINNET.id() {
                Ok("mainnet".to_string())
            } else {
                Err(anyhow!(
                    "paymaster sidecar requires SN_MAIN or SN_SEPOLIA chain id, got {id:#x}"
                ))
            }
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
            // Check if the id matches known chain IDs
            if id == ChainId::SEPOLIA.id() {
                Ok(DEFAULT_AVNU_PRICE_SEPOLIA_ENDPOINT)
            } else if id == ChainId::MAINNET.id() {
                Ok(DEFAULT_AVNU_PRICE_MAINNET_ENDPOINT)
            } else {
                Err(anyhow!(
                    "paymaster sidecar requires SN_MAIN or SN_SEPOLIA chain id, got {id:#x}"
                ))
            }
        }
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
