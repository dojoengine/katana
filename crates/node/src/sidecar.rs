use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::{env, fs};

use anyhow::{anyhow, Context, Result};
#[cfg(feature = "vrf")]
use ark_ff::PrimeField;
use katana_core::backend::Backend;
use katana_core::service::block_producer::BlockProducer;
use katana_executor::ExecutorFactory;
use katana_genesis::allocation::GenesisAccountAlloc;
use katana_genesis::constant::{
    DEFAULT_ETH_FEE_TOKEN_ADDRESS, DEFAULT_STRK_FEE_TOKEN_ADDRESS, DEFAULT_UDC_ADDRESS,
};
use katana_pool::TxPool;
use katana_pool_api::TransactionPool;
use katana_primitives::chain::{ChainId, NamedChainId};
use katana_primitives::da::DataAvailabilityMode;
use katana_primitives::fee::{AllResourceBoundsMapping, ResourceBoundsMapping};
use katana_primitives::transaction::{ExecutableTx, ExecutableTxWithHash, InvokeTx, InvokeTxV3};
use katana_primitives::utils::get_contract_address;
#[cfg(feature = "vrf")]
use katana_primitives::utils::split_u256;
#[cfg(feature = "vrf")]
use katana_primitives::U256;
use katana_primitives::{ContractAddress, Felt};
use katana_provider::api::state::{StateFactoryProvider, StateProvider};
use katana_provider::ProviderFactory;
use katana_rpc_types::FunctionCall;
use serde::Serialize;
#[cfg(feature = "vrf")]
use stark_vrf::{generate_public_key, ScalarField};
use starknet::macros::selector;
use starknet::signers::{LocalWallet, Signer, SigningKey};
use tokio::process::{Child, Command};
use tokio::time::sleep;
use tracing::{debug, info, warn};
use url::Url;

use crate::config::paymaster::{PaymasterConfig, ServiceMode};
#[cfg(feature = "vrf")]
use crate::config::paymaster::{VrfConfig, VrfKeySource};
use crate::config::Config;

const FORWARDER_SALT: u64 = 0x12345;
#[cfg(feature = "vrf")]
const VRF_ACCOUNT_SALT: u64 = 0x54321;
#[cfg(feature = "vrf")]
const VRF_CONSUMER_SALT: u64 = 0x67890;
const BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(feature = "vrf")]
const VRF_SERVER_PORT: u16 = 3000;
const DEFAULT_AVNU_PRICE_SEPOLIA_ENDPOINT: &str = "https://sepolia.api.avnu.fi";
const DEFAULT_AVNU_PRICE_MAINNET_ENDPOINT: &str = "https://starknet.api.avnu.fi";

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

#[derive(Debug, Clone)]
pub struct PaymasterBootstrap {
    pub forwarder_address: ContractAddress,
    pub relayer_address: ContractAddress,
    pub relayer_private_key: Felt,
    pub gas_tank_address: ContractAddress,
    pub gas_tank_private_key: Felt,
    pub estimate_account_address: ContractAddress,
    pub estimate_account_private_key: Felt,
    pub chain_id: ChainId,
}

#[cfg(feature = "vrf")]
#[derive(Debug, Clone)]
pub struct VrfBootstrap {
    pub secret_key: u64,
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

#[derive(Debug, Default)]
pub struct BootstrapResult {
    pub paymaster: Option<PaymasterBootstrap>,
    #[cfg(feature = "vrf")]
    pub vrf: Option<VrfBootstrap>,
}

pub async fn bootstrap_sidecars<EF, PF>(
    config: &Config,
    backend: &Backend<EF, PF>,
    block_producer: &BlockProducer<EF, PF>,
    pool: &TxPool,
) -> Result<BootstrapResult>
where
    EF: ExecutorFactory,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: katana_core::backend::storage::ProviderRO,
    <PF as ProviderFactory>::ProviderMut: katana_core::backend::storage::ProviderRW,
{
    let mut result = BootstrapResult::default();

    if let Some(paymaster_cfg) = config.paymaster.as_ref() {
        if paymaster_cfg.mode == ServiceMode::Sidecar {
            let bootstrap =
                bootstrap_paymaster(paymaster_cfg, backend, block_producer, pool).await?;
            result.paymaster = Some(bootstrap);
        }
    }

    #[cfg(feature = "vrf")]
    if let Some(vrf_cfg) = config.vrf.as_ref() {
        if vrf_cfg.mode == ServiceMode::Sidecar {
            let bootstrap = bootstrap_vrf(vrf_cfg, config, backend, block_producer, pool).await?;
            result.vrf = Some(bootstrap);
        }
    }

    Ok(result)
}

pub async fn start_sidecars(
    config: &Config,
    bootstrap: &BootstrapResult,
    rpc_addr: &std::net::SocketAddr,
) -> Result<SidecarProcesses> {
    let mut paymaster_child = None;
    #[cfg(feature = "vrf")]
    let mut vrf_child = None;
    if let (Some(paymaster_cfg), Some(paymaster_bootstrap)) =
        (config.paymaster.as_ref(), bootstrap.paymaster.as_ref())
    {
        if paymaster_cfg.mode == ServiceMode::Sidecar {
            paymaster_child =
                Some(start_paymaster_sidecar(paymaster_cfg, paymaster_bootstrap, rpc_addr).await?);
        }
    }

    #[cfg(feature = "vrf")]
    if let (Some(vrf_cfg), Some(vrf_bootstrap)) = (config.vrf.as_ref(), bootstrap.vrf.as_ref()) {
        if vrf_cfg.mode == ServiceMode::Sidecar {
            vrf_child = Some(start_vrf_sidecar(vrf_cfg, vrf_bootstrap).await?);
        }
    }

    #[cfg(feature = "vrf")]
    let processes = SidecarProcesses::new(paymaster_child, vrf_child);
    #[cfg(not(feature = "vrf"))]
    let processes = SidecarProcesses::new(paymaster_child);

    Ok(processes)
}

async fn start_paymaster_sidecar(
    config: &PaymasterConfig,
    bootstrap: &PaymasterBootstrap,
    rpc_addr: &std::net::SocketAddr,
) -> Result<Child> {
    let bin = config.sidecar_bin.clone().unwrap_or_else(|| "paymaster-service".into());
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

    wait_for_paymaster_ready(&config.url, config.api_key.as_deref(), BOOTSTRAP_TIMEOUT).await?;

    Ok(child)
}

#[cfg(feature = "vrf")]
async fn start_vrf_sidecar(config: &VrfConfig, bootstrap: &VrfBootstrap) -> Result<Child> {
    if config.sidecar_port != VRF_SERVER_PORT {
        return Err(anyhow!(
            "vrf-server uses a fixed port of {VRF_SERVER_PORT}; set --vrf.port={VRF_SERVER_PORT} \
             or use --vrf.mode=external"
        ));
    }

    let bin = config.sidecar_bin.clone().unwrap_or_else(|| "vrf-server".into());
    let bin = resolve_executable(Path::new(&bin))?;

    let mut command = Command::new(bin);
    command
        .arg("--secret-key")
        .arg(bootstrap.secret_key.to_string())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);

    let child = command.spawn().context("failed to spawn vrf sidecar")?;

    wait_for_http_ok(&format!("{}/info", config.url), "vrf info", BOOTSTRAP_TIMEOUT).await?;

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

fn build_paymaster_profile(
    config: &PaymasterConfig,
    bootstrap: &PaymasterBootstrap,
    rpc_url: &Url,
) -> Result<PaymasterProfile> {
    let chain_id = paymaster_chain_id(bootstrap.chain_id)?;
    let api_key = config.api_key.clone().unwrap_or_else(|| "paymaster_katana".to_string());
    let price_endpoint = paymaster_price_endpoint(bootstrap.chain_id)?;
    let price_api_key = config.price_api_key.clone().unwrap_or_default();

    let eth_token = format_felt(DEFAULT_ETH_FEE_TOKEN_ADDRESS.into());
    let strk_token = format_felt(DEFAULT_STRK_FEE_TOKEN_ADDRESS.into());

    Ok(PaymasterProfile {
        verbosity: "info".to_string(),
        prometheus: None,
        rpc: PaymasterRpcProfile { port: config.sidecar_port as u64 },
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
            api_key,
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

fn local_rpc_url(addr: &std::net::SocketAddr) -> Url {
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

fn format_felt(value: Felt) -> String {
    format!("{value:#x}")
}

#[cfg(feature = "vrf")]
fn scalar_from_felt(value: Felt) -> ScalarField {
    let bytes = value.to_bytes_be();
    ScalarField::from_be_bytes_mod_order(&bytes)
}

#[cfg(feature = "vrf")]
fn vrf_secret_key_from_account_key(value: Felt) -> u64 {
    let bytes = value.to_bytes_be();
    let mut tail = [0_u8; 8];
    tail.copy_from_slice(&bytes[24..]);
    u64::from_be_bytes(tail)
}

#[cfg(feature = "vrf")]
fn felt_from_field<T: std::fmt::Display>(value: T) -> Result<Felt> {
    let decimal = value.to_string();
    Felt::from_dec_str(&decimal).map_err(|err| anyhow!("invalid field value: {err}"))
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

async fn bootstrap_paymaster<EF, PF>(
    config: &PaymasterConfig,
    backend: &Backend<EF, PF>,
    block_producer: &BlockProducer<EF, PF>,
    pool: &TxPool,
) -> Result<PaymasterBootstrap>
where
    EF: ExecutorFactory,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: katana_core::backend::storage::ProviderRO,
    <PF as ProviderFactory>::ProviderMut: katana_core::backend::storage::ProviderRW,
{
    let (relayer_address, relayer_private_key) =
        prefunded_account(backend, config.prefunded_index)?;
    let gas_tank_index = config
        .prefunded_index
        .checked_add(1)
        .ok_or_else(|| anyhow!("paymaster gas tank index overflow"))?;
    let estimate_index = config
        .prefunded_index
        .checked_add(2)
        .ok_or_else(|| anyhow!("paymaster estimate index overflow"))?;

    let (gas_tank_address, gas_tank_private_key) = prefunded_account(backend, gas_tank_index)?;
    let (estimate_account_address, estimate_account_private_key) =
        prefunded_account(backend, estimate_index)?;

    let forwarder_class_hash = avnu_forwarder_class_hash()?;
    let forwarder_address = get_contract_address(
        Felt::from(FORWARDER_SALT),
        forwarder_class_hash,
        &[relayer_address.into(), gas_tank_address.into()],
        DEFAULT_UDC_ADDRESS.into(),
    )
    .into();

    ensure_deployed(
        backend,
        block_producer,
        pool,
        DeploymentRequest {
            sender_address: relayer_address,
            sender_private_key: relayer_private_key,
            target_address: forwarder_address,
            class_hash: forwarder_class_hash,
            constructor_calldata: vec![relayer_address.into(), gas_tank_address.into()],
            salt: Felt::from(FORWARDER_SALT),
        },
    )
    .await?;

    let whitelist_call = FunctionCall {
        contract_address: forwarder_address,
        entry_point_selector: selector!("set_whitelisted_address"),
        calldata: vec![relayer_address.into(), Felt::ONE],
    };

    submit_invoke(
        backend,
        block_producer,
        pool,
        relayer_address,
        relayer_private_key,
        vec![whitelist_call],
    )
    .await?;

    let chain_id = backend.chain_spec.id();

    Ok(PaymasterBootstrap {
        forwarder_address,
        relayer_address,
        relayer_private_key,
        gas_tank_address,
        gas_tank_private_key,
        estimate_account_address,
        estimate_account_private_key,
        chain_id,
    })
}

#[cfg(feature = "vrf")]
#[derive(Debug)]
pub(crate) struct VrfDerivedAccounts {
    pub(crate) source_address: ContractAddress,
    pub(crate) source_private_key: Felt,
    pub(crate) vrf_account_address: ContractAddress,
    pub(crate) vrf_public_key_x: Felt,
    pub(crate) vrf_public_key_y: Felt,
    pub(crate) secret_key: u64,
}

#[cfg(feature = "vrf")]
pub(crate) fn derive_vrf_accounts<EF, PF>(
    config: &VrfConfig,
    node_config: &Config,
    backend: &Backend<EF, PF>,
) -> Result<VrfDerivedAccounts>
where
    EF: ExecutorFactory,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: katana_core::backend::storage::ProviderRO,
    <PF as ProviderFactory>::ProviderMut: katana_core::backend::storage::ProviderRW,
{
    let (source_address, source_private_key) = match config.key_source {
        VrfKeySource::Prefunded => prefunded_account(backend, config.prefunded_index)?,
        VrfKeySource::Sequencer => sequencer_account(node_config, backend)?,
    };

    // vrf-server expects a u64 secret, so derive one from the account key.
    let secret_key = vrf_secret_key_from_account_key(source_private_key);
    let public_key = generate_public_key(scalar_from_felt(Felt::from(secret_key)));
    let vrf_public_key_x = felt_from_field(public_key.x)?;
    let vrf_public_key_y = felt_from_field(public_key.y)?;

    let account_public_key =
        SigningKey::from_secret_scalar(source_private_key).verifying_key().scalar();
    let vrf_account_class_hash = vrf_account_class_hash()?;
    let vrf_account_address = get_contract_address(
        Felt::from(VRF_ACCOUNT_SALT),
        vrf_account_class_hash,
        &[account_public_key],
        DEFAULT_UDC_ADDRESS.into(),
    )
    .into();

    Ok(VrfDerivedAccounts {
        source_address,
        source_private_key,
        vrf_account_address,
        vrf_public_key_x,
        vrf_public_key_y,
        secret_key,
    })
}

#[cfg(feature = "vrf")]
async fn bootstrap_vrf<EF, PF>(
    config: &VrfConfig,
    node_config: &Config,
    backend: &Backend<EF, PF>,
    block_producer: &BlockProducer<EF, PF>,
    pool: &TxPool,
) -> Result<VrfBootstrap>
where
    EF: ExecutorFactory,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: katana_core::backend::storage::ProviderRO,
    <PF as ProviderFactory>::ProviderMut: katana_core::backend::storage::ProviderRW,
{
    let derived = derive_vrf_accounts(config, node_config, backend)?;
    let account_address = derived.source_address;
    let account_private_key = derived.source_private_key;
    let vrf_account_address = derived.vrf_account_address;
    let account_public_key =
        SigningKey::from_secret_scalar(account_private_key).verifying_key().scalar();
    let vrf_account_class_hash = vrf_account_class_hash()?;

    ensure_deployed(
        backend,
        block_producer,
        pool,
        DeploymentRequest {
            sender_address: account_address,
            sender_private_key: account_private_key,
            target_address: vrf_account_address,
            class_hash: vrf_account_class_hash,
            constructor_calldata: vec![account_public_key],
            salt: Felt::from(VRF_ACCOUNT_SALT),
        },
    )
    .await?;

    if node_config.dev.fee {
        fund_account(
            backend,
            block_producer,
            pool,
            account_address,
            account_private_key,
            vrf_account_address,
        )
        .await?;
    }

    let set_vrf_key_call = FunctionCall {
        contract_address: vrf_account_address,
        entry_point_selector: selector!("set_vrf_public_key"),
        calldata: vec![derived.vrf_public_key_x, derived.vrf_public_key_y],
    };

    submit_invoke(
        backend,
        block_producer,
        pool,
        vrf_account_address,
        account_private_key,
        vec![set_vrf_key_call],
    )
    .await?;

    let vrf_consumer_class_hash = vrf_consumer_class_hash()?;
    let vrf_consumer_address = get_contract_address(
        Felt::from(VRF_CONSUMER_SALT),
        vrf_consumer_class_hash,
        &[vrf_account_address.into()],
        DEFAULT_UDC_ADDRESS.into(),
    )
    .into();

    ensure_deployed(
        backend,
        block_producer,
        pool,
        DeploymentRequest {
            sender_address: account_address,
            sender_private_key: account_private_key,
            target_address: vrf_consumer_address,
            class_hash: vrf_consumer_class_hash,
            constructor_calldata: vec![vrf_account_address.into()],
            salt: Felt::from(VRF_CONSUMER_SALT),
        },
    )
    .await?;

    Ok(VrfBootstrap { secret_key: derived.secret_key })
}

fn prefunded_account<EF, PF>(
    backend: &Backend<EF, PF>,
    index: u16,
) -> Result<(ContractAddress, Felt)>
where
    EF: ExecutorFactory,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: katana_core::backend::storage::ProviderRO,
    <PF as ProviderFactory>::ProviderMut: katana_core::backend::storage::ProviderRW,
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

#[cfg(feature = "vrf")]
fn sequencer_account<EF, PF>(
    config: &Config,
    backend: &Backend<EF, PF>,
) -> Result<(ContractAddress, Felt)>
where
    EF: ExecutorFactory,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: katana_core::backend::storage::ProviderRO,
    <PF as ProviderFactory>::ProviderMut: katana_core::backend::storage::ProviderRW,
{
    let sequencer = config.chain.genesis().sequencer_address;

    for (address, allocation) in backend.chain_spec.genesis().accounts() {
        if *address == sequencer {
            let private_key = match allocation {
                GenesisAccountAlloc::DevAccount(account) => account.private_key,
                _ => return Err(anyhow!("sequencer account has no private key")),
            };
            return Ok((*address, private_key));
        }
    }

    Err(anyhow!("sequencer key source requested but sequencer is not a prefunded account"))
}

struct DeploymentRequest {
    sender_address: ContractAddress,
    sender_private_key: Felt,
    target_address: ContractAddress,
    class_hash: Felt,
    constructor_calldata: Vec<Felt>,
    salt: Felt,
}

async fn ensure_deployed<EF, PF>(
    backend: &Backend<EF, PF>,
    block_producer: &BlockProducer<EF, PF>,
    pool: &TxPool,
    request: DeploymentRequest,
) -> Result<()>
where
    EF: ExecutorFactory,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: katana_core::backend::storage::ProviderRO,
    <PF as ProviderFactory>::ProviderMut: katana_core::backend::storage::ProviderRW,
{
    let DeploymentRequest {
        sender_address,
        sender_private_key,
        target_address,
        class_hash,
        constructor_calldata,
        salt,
    } = request;

    if is_deployed(backend, target_address)? {
        return Ok(());
    }

    let deploy_call = FunctionCall {
        contract_address: DEFAULT_UDC_ADDRESS,
        entry_point_selector: selector!("deployContract"),
        calldata: udc_calldata(class_hash, salt, constructor_calldata),
    };

    submit_invoke(
        backend,
        block_producer,
        pool,
        sender_address,
        sender_private_key,
        vec![deploy_call],
    )
    .await?;

    wait_for_contract(backend, target_address, BOOTSTRAP_TIMEOUT).await?;
    Ok(())
}

#[cfg(feature = "vrf")]
async fn fund_account<EF, PF>(
    backend: &Backend<EF, PF>,
    block_producer: &BlockProducer<EF, PF>,
    pool: &TxPool,
    sender_address: ContractAddress,
    sender_private_key: Felt,
    recipient: ContractAddress,
) -> Result<()>
where
    EF: ExecutorFactory,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: katana_core::backend::storage::ProviderRO,
    <PF as ProviderFactory>::ProviderMut: katana_core::backend::storage::ProviderRW,
{
    let amount = Felt::from(1_000_000_000_000_000_000u128);
    let (low, high) = split_u256(U256::from_be_bytes(amount.to_bytes_be()));

    let transfer_call = FunctionCall {
        contract_address: DEFAULT_STRK_FEE_TOKEN_ADDRESS,
        entry_point_selector: selector!("transfer"),
        calldata: vec![recipient.into(), low, high],
    };

    submit_invoke(
        backend,
        block_producer,
        pool,
        sender_address,
        sender_private_key,
        vec![transfer_call],
    )
    .await
}

async fn submit_invoke<EF, PF>(
    backend: &Backend<EF, PF>,
    block_producer: &BlockProducer<EF, PF>,
    pool: &TxPool,
    sender_address: ContractAddress,
    sender_private_key: Felt,
    calls: Vec<FunctionCall>,
) -> Result<()>
where
    EF: ExecutorFactory,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: katana_core::backend::storage::ProviderRO,
    <PF as ProviderFactory>::ProviderMut: katana_core::backend::storage::ProviderRW,
{
    let state = backend.storage.provider().latest()?;
    let nonce = account_nonce(pool, state.as_ref(), sender_address)?;

    let tx =
        sign_invoke_tx(backend.chain_spec.id(), sender_address, sender_private_key, nonce, calls)?;

    pool.add_transaction(tx)
        .await
        .map_err(|err| anyhow!("failed to add transaction to pool: {err}"))?;
    block_producer.force_mine();

    Ok(())
}

fn account_nonce(
    pool: &TxPool,
    state: &dyn StateProvider,
    address: ContractAddress,
) -> Result<Felt> {
    if let Some(nonce) = pool.get_nonce(address) {
        return Ok(nonce);
    }
    Ok(state.nonce(address)?.unwrap_or_default())
}

fn sign_invoke_tx(
    chain_id: katana_primitives::chain::ChainId,
    sender_address: ContractAddress,
    sender_private_key: Felt,
    nonce: Felt,
    calls: Vec<FunctionCall>,
) -> Result<ExecutableTxWithHash> {
    let mut tx = InvokeTxV3 {
        nonce,
        chain_id,
        calldata: encode_calls(calls),
        signature: vec![],
        sender_address,
        tip: 0_u64,
        paymaster_data: vec![],
        account_deployment_data: vec![],
        nonce_data_availability_mode: DataAvailabilityMode::L1,
        fee_data_availability_mode: DataAvailabilityMode::L1,
        resource_bounds: ResourceBoundsMapping::All(AllResourceBoundsMapping::default()),
    };

    let tx_hash = InvokeTx::V3(tx.clone()).calculate_hash(false);

    let signer = LocalWallet::from(SigningKey::from_secret_scalar(sender_private_key));
    let signature =
        futures::executor::block_on(signer.sign_hash(&tx_hash)).map_err(|e| anyhow!(e))?;
    tx.signature = vec![signature.r, signature.s];

    let tx = ExecutableTxWithHash::new(ExecutableTx::Invoke(InvokeTx::V3(tx)));

    Ok(tx)
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

fn is_deployed<EF, PF>(backend: &Backend<EF, PF>, address: ContractAddress) -> Result<bool>
where
    EF: ExecutorFactory,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: katana_core::backend::storage::ProviderRO,
    <PF as ProviderFactory>::ProviderMut: katana_core::backend::storage::ProviderRW,
{
    let state = backend.storage.provider().latest()?;
    Ok(state.class_hash_of_contract(address)?.is_some())
}

async fn wait_for_contract<EF, PF>(
    backend: &Backend<EF, PF>,
    address: ContractAddress,
    timeout: Duration,
) -> Result<()>
where
    EF: ExecutorFactory,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: katana_core::backend::storage::ProviderRO,
    <PF as ProviderFactory>::ProviderMut: katana_core::backend::storage::ProviderRW,
{
    let start = Instant::now();
    loop {
        if is_deployed(backend, address)? {
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

#[cfg(feature = "vrf")]
fn vrf_account_class_hash() -> Result<Felt> {
    let class = katana_primitives::utils::class::parse_sierra_class(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../controller/classes/cartridge_vrf_VrfAccount.contract_class.json"
    )))?;
    class.class_hash().context("failed to compute vrf account class hash")
}

#[cfg(feature = "vrf")]
fn vrf_consumer_class_hash() -> Result<Felt> {
    let class = katana_primitives::utils::class::parse_sierra_class(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../controller/classes/cartridge_vrf_VrfConsumer.contract_class.json"
    )))?;
    class.class_hash().context("failed to compute vrf consumer class hash")
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::path::Path;
    use std::sync::Mutex;

    use katana_primitives::chain::{ChainId, NamedChainId};
    use katana_primitives::Felt;
    use tempfile::tempdir;
    use url::Url;

    #[cfg(feature = "vrf")]
    use super::vrf_secret_key_from_account_key;
    use super::{
        build_paymaster_profile, local_rpc_url, paymaster_chain_id, paymaster_price_endpoint,
        resolve_executable, PaymasterBootstrap, DEFAULT_AVNU_PRICE_MAINNET_ENDPOINT,
        DEFAULT_AVNU_PRICE_SEPOLIA_ENDPOINT,
    };
    use crate::config::paymaster::{PaymasterConfig, ServiceMode};

    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    #[cfg(feature = "vrf")]
    #[test]
    fn vrf_secret_key_uses_low_64_bits() {
        let mut bytes = [0_u8; 32];
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = i as u8;
        }

        let felt = Felt::from_bytes_be(&bytes);
        let secret = vrf_secret_key_from_account_key(felt);

        assert_eq!(secret, 0x18191a1b1c1d1e1f);
    }

    #[test]
    fn local_rpc_url_rewrites_unspecified_host() {
        let addr: std::net::SocketAddr = "0.0.0.0:5050".parse().expect("socket addr");
        let url = local_rpc_url(&addr);
        assert_eq!(url.as_str(), "http://127.0.0.1:5050/");
    }

    #[test]
    fn paymaster_chain_id_rejects_unknown_chain() {
        let err = paymaster_chain_id(ChainId::Named(NamedChainId::Goerli)).unwrap_err();
        assert!(
            err.to_string().contains("only supports SN_MAIN or SN_SEPOLIA"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn paymaster_price_endpoint_defaults() {
        let sepolia = paymaster_price_endpoint(ChainId::Named(NamedChainId::Sepolia)).unwrap();
        assert_eq!(sepolia, DEFAULT_AVNU_PRICE_SEPOLIA_ENDPOINT);

        let mainnet = paymaster_price_endpoint(ChainId::Named(NamedChainId::Mainnet)).unwrap();
        assert_eq!(mainnet, DEFAULT_AVNU_PRICE_MAINNET_ENDPOINT);
    }

    #[test]
    fn build_paymaster_profile_defaults() {
        let config = PaymasterConfig {
            mode: ServiceMode::Sidecar,
            url: Url::parse("http://127.0.0.1:4337").unwrap(),
            api_key: None,
            price_api_key: Some("price_key".to_string()),
            prefunded_index: 0,
            sidecar_port: 4337,
            sidecar_bin: None,
            #[cfg(feature = "cartridge")]
            cartridge_api_url: None,
        };

        let bootstrap = PaymasterBootstrap {
            forwarder_address: Felt::from(0x11_u64).into(),
            relayer_address: Felt::from(0x22_u64).into(),
            relayer_private_key: Felt::from(0x33_u64),
            gas_tank_address: Felt::from(0x44_u64).into(),
            gas_tank_private_key: Felt::from(0x55_u64),
            estimate_account_address: Felt::from(0x66_u64).into(),
            estimate_account_private_key: Felt::from(0x77_u64),
            chain_id: ChainId::Named(NamedChainId::Sepolia),
        };

        let rpc_url = Url::parse("http://127.0.0.1:5050").unwrap();
        let profile = build_paymaster_profile(&config, &bootstrap, &rpc_url).unwrap();

        assert_eq!(profile.rpc.port, 4337);
        assert_eq!(profile.forwarder, format!("{:#x}", Felt::from(0x11_u64)));
        assert_eq!(profile.sponsoring.api_key, "paymaster_katana");
        assert_eq!(profile.price.endpoint, DEFAULT_AVNU_PRICE_SEPOLIA_ENDPOINT);
        assert_eq!(profile.price.api_key, "price_key");
        assert_eq!(profile.starknet.endpoint, rpc_url.to_string());
    }

    #[test]
    fn resolve_executable_with_explicit_path() {
        let dir = tempdir().expect("tempdir");
        let bin_path = dir.path().join("sidecar-bin");
        std::fs::write(&bin_path, "binary").expect("write bin");

        let resolved = resolve_executable(&bin_path).expect("resolve path");
        assert_eq!(resolved, bin_path);
    }

    #[test]
    fn resolve_executable_searches_path() {
        let _guard = ENV_MUTEX.lock().expect("env mutex");
        let dir = tempdir().expect("tempdir");
        let bin_path = dir.path().join("sidecar-bin");
        std::fs::write(&bin_path, "binary").expect("write bin");

        let old_path = env::var_os("PATH");
        let mut paths = vec![dir.path().to_path_buf()];
        if let Some(old) = old_path.as_ref() {
            paths.extend(env::split_paths(old));
        }
        let new_path = env::join_paths(paths).expect("join paths");
        env::set_var("PATH", &new_path);

        let resolved = resolve_executable(Path::new("sidecar-bin")).expect("resolve path search");
        assert_eq!(resolved, bin_path);

        match old_path {
            Some(value) => env::set_var("PATH", value),
            None => env::remove_var("PATH"),
        }
    }
}
