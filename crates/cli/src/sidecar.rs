//! Sidecar bootstrap and process management for CLI.
//!
//! This module handles:
//! - Building paymaster and VRF configurations from CLI options
//! - Bootstrapping paymaster and VRF services (deploying contracts)
//! - Spawning and managing sidecar processes when running in sidecar mode
//!
//! The node treats all paymaster/VRF services as external - this module bridges
//! the gap by deploying necessary contracts and spawning sidecar processes.

#[cfg(feature = "vrf")]
use std::env;
use std::net::SocketAddr;
#[cfg(feature = "vrf")]
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

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
#[cfg(feature = "paymaster")]
use katana_node::config::paymaster::PaymasterConfig;
#[cfg(feature = "vrf")]
use katana_node::config::paymaster::{VrfConfig, VrfKeySource as NodeVrfKeySource};
pub use katana_paymaster::{
    bootstrap_paymaster, format_felt, start_paymaster_sidecar, wait_for_paymaster_ready,
    PaymasterBootstrapConfig, PaymasterBootstrapResult, PaymasterSidecarConfig,
};
use katana_pool::TxPool;
#[cfg(feature = "vrf")]
use katana_pool_api::TransactionPool;
use katana_primitives::chain::ChainId;
#[cfg(feature = "vrf")]
use katana_primitives::da::DataAvailabilityMode;
#[cfg(feature = "vrf")]
use katana_primitives::fee::{AllResourceBoundsMapping, ResourceBoundsMapping};
#[cfg(feature = "vrf")]
use katana_primitives::transaction::{ExecutableTx, ExecutableTxWithHash, InvokeTx, InvokeTxV3};
use katana_primitives::utils::get_contract_address;
#[cfg(feature = "vrf")]
use katana_primitives::utils::split_u256;
#[cfg(feature = "vrf")]
use katana_primitives::U256;
use katana_primitives::{ContractAddress, Felt};
#[cfg(feature = "vrf")]
use katana_provider::api::state::{StateFactoryProvider, StateProvider};
use katana_provider::ProviderFactory;
#[cfg(feature = "vrf")]
use katana_rpc_types::FunctionCall;
#[cfg(feature = "vrf")]
use stark_vrf::{generate_public_key, ScalarField};
#[cfg(feature = "vrf")]
use starknet::macros::selector;
#[cfg(feature = "vrf")]
use starknet::signers::{LocalWallet, Signer, SigningKey};
use tokio::process::Child;
#[cfg(feature = "vrf")]
use tokio::process::Command;
use tokio::time::sleep;
use tracing::{debug, info, warn};
use url::Url;

use crate::options::PaymasterOptions;
#[cfg(feature = "vrf")]
use crate::options::{VrfKeySource as OptionsVrfKeySource, VrfOptions};

const LOG_TARGET: &str = "katana::cli::sidecar";

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

/// Sidecar-specific info for VRF (used by CLI to start sidecar process).
#[cfg(feature = "vrf")]
#[derive(Debug, Clone)]
pub struct VrfSidecarInfo {
    pub port: u16,
}

// ============================================================================
// Config Building Functions
// ============================================================================

/// Build the paymaster configuration from CLI options.
///
/// Returns `None` if paymaster is not enabled.
/// Returns `(PaymasterConfig, Option<PaymasterSidecarInfo>)` where the sidecar info
/// is `Some` in sidecar mode and `None` in external mode.
#[cfg(feature = "paymaster")]
pub fn build_paymaster_config(
    options: &PaymasterOptions,
    #[cfg(feature = "cartridge")] cartridge_api_url: &url::Url,
) -> Result<Option<(PaymasterConfig, Option<PaymasterSidecarInfo>)>> {
    if !options.is_enabled() {
        return Ok(None);
    }

    // Determine mode based on whether URL is provided
    let is_external = options.is_external();

    // For sidecar mode, allocate a free port and prepare sidecar info
    let (url, sidecar_info) = if is_external {
        // External mode: use the provided URL
        let url = options.url.clone().expect("URL must be set in external mode");
        (url, None)
    } else {
        // Sidecar mode: allocate a free port
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .context("failed to find free port for paymaster sidecar")?;
        let port = listener.local_addr()?.port();
        let url = Url::parse(&format!("http://127.0.0.1:{port}")).expect("valid url");

        // Validate and prepare API key
        let api_key = {
            let key =
                options.api_key.clone().unwrap_or_else(|| DEFAULT_PAYMASTER_API_KEY.to_string());
            if !key.starts_with("paymaster_") {
                warn!(
                    target: LOG_TARGET,
                    %key,
                    "paymaster api key must start with 'paymaster_'; using default"
                );
                DEFAULT_PAYMASTER_API_KEY.to_string()
            } else {
                key
            }
        };

        let sidecar_info = PaymasterSidecarInfo { port, api_key };
        (url, Some(sidecar_info))
    };

    let api_key = if is_external {
        options.api_key.clone()
    } else {
        sidecar_info.as_ref().map(|s| s.api_key.clone())
    };

    let config = PaymasterConfig {
        url,
        api_key,
        prefunded_index: options.prefunded_index,
        #[cfg(feature = "cartridge")]
        cartridge_api_url: Some(cartridge_api_url.clone()),
    };

    Ok(Some((config, sidecar_info)))
}

/// Build the VRF configuration from CLI options.
///
/// Returns `None` if VRF is not enabled.
/// Returns `(VrfConfig, Option<VrfSidecarInfo>)` where the sidecar info
/// is `Some` in sidecar mode and `None` in external mode.
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

    let config = VrfConfig { url, key_source, prefunded_index: options.prefunded_index };

    Ok(Some((config, sidecar_info)))
}

// ============================================================================
// Bootstrap Types
// ============================================================================

#[cfg(feature = "vrf")]
const VRF_ACCOUNT_SALT: u64 = 0x54321;
#[cfg(feature = "vrf")]
const VRF_CONSUMER_SALT: u64 = 0x67890;
const BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(feature = "vrf")]
const VRF_SERVER_PORT: u16 = 3000;

// ============================================================================
// Bootstrap Types
// ============================================================================

/// Bootstrap data for the VRF service.
#[cfg(feature = "vrf")]
#[derive(Debug, Clone)]
pub struct VrfBootstrap {
    pub secret_key: u64,
}

/// Result of bootstrapping sidecars.
#[derive(Debug, Default)]
pub struct BootstrapResult {
    pub paymaster: Option<PaymasterBootstrapInfo>,
    #[cfg(feature = "vrf")]
    pub vrf: Option<VrfBootstrap>,
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
    /// Index of the first prefunded account to use (relayer).
    pub prefunded_index: u16,
}

/// VRF-specific bootstrap configuration.
#[cfg(feature = "vrf")]
pub struct VrfBootstrapConfig {
    pub key_source: VrfKeySource,
    pub prefunded_index: u16,
    pub sequencer_address: ContractAddress,
}

/// Source of the VRF key.
#[cfg(feature = "vrf")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VrfKeySource {
    Prefunded,
    Sequencer,
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
    block_producer: &BlockProducer<EF, PF>,
    pool: &TxPool,
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
        let bootstrap =
            bootstrap_vrf(vrf_cfg, config.fee_enabled, backend, block_producer, pool).await?;
        result.vrf = Some(bootstrap);
    }

    Ok(result)
}

/// Bootstrap the paymaster using genesis accounts from the backend.
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
    // Extract account info from genesis
    let (relayer_address, relayer_private_key) = prefunded_account(backend, input.prefunded_index)?;

    let gas_tank_index = input
        .prefunded_index
        .checked_add(1)
        .ok_or_else(|| anyhow!("paymaster gas tank index overflow"))?;
    let estimate_index = input
        .prefunded_index
        .checked_add(2)
        .ok_or_else(|| anyhow!("paymaster estimate index overflow"))?;

    let (gas_tank_address, gas_tank_private_key) = prefunded_account(backend, gas_tank_index)?;
    let (estimate_account_address, estimate_account_private_key) =
        prefunded_account(backend, estimate_index)?;

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

#[cfg(feature = "vrf")]
#[derive(Debug)]
pub struct VrfDerivedAccounts {
    pub source_address: ContractAddress,
    pub source_private_key: Felt,
    pub vrf_account_address: ContractAddress,
    pub vrf_public_key_x: Felt,
    pub vrf_public_key_y: Felt,
    pub secret_key: u64,
}

#[cfg(feature = "vrf")]
pub fn derive_vrf_accounts<EF, PF>(
    config: &VrfBootstrapConfig,
    backend: &Backend<EF, PF>,
) -> Result<VrfDerivedAccounts>
where
    EF: ExecutorFactory,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: katana_provider::ProviderRO,
    <PF as ProviderFactory>::ProviderMut: katana_provider::ProviderRW,
{
    let (source_address, source_private_key) = match config.key_source {
        VrfKeySource::Prefunded => prefunded_account(backend, config.prefunded_index)?,
        VrfKeySource::Sequencer => sequencer_account(config.sequencer_address, backend)?,
    };

    // vrf-server expects a u64 secret, so derive one from the account key.
    let secret_key = vrf_secret_key_from_account_key(source_private_key);
    let public_key = generate_public_key(scalar_from_felt(Felt::from(secret_key)));
    let vrf_public_key_x = felt_from_field(public_key.x)?;
    let vrf_public_key_y = felt_from_field(public_key.y)?;

    let account_public_key =
        SigningKey::from_secret_scalar(source_private_key).verifying_key().scalar();
    let vrf_account_class_hash = vrf_account_class_hash()?;
    // When using UDC with unique=0 (non-unique deployment), the deployer_address
    // used in address computation is 0, not the actual deployer or UDC address.
    let vrf_account_address = get_contract_address(
        Felt::from(VRF_ACCOUNT_SALT),
        vrf_account_class_hash,
        &[account_public_key],
        Felt::ZERO,
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
    config: &VrfBootstrapConfig,
    fee_enabled: bool,
    backend: &Backend<EF, PF>,
    block_producer: &BlockProducer<EF, PF>,
    pool: &TxPool,
) -> Result<VrfBootstrap>
where
    EF: ExecutorFactory,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: katana_provider::ProviderRO,
    <PF as ProviderFactory>::ProviderMut: katana_provider::ProviderRW,
{
    let derived = derive_vrf_accounts(config, backend)?;
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

    if fee_enabled {
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
    // When using UDC with unique=0 (non-unique deployment), the deployer_address
    // used in address computation is 0, not the actual deployer or UDC address.
    let vrf_consumer_address = get_contract_address(
        Felt::from(VRF_CONSUMER_SALT),
        vrf_consumer_class_hash,
        &[vrf_account_address.into()],
        Felt::ZERO,
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

#[cfg(feature = "vrf")]
fn sequencer_account<EF, PF>(
    sequencer_address: ContractAddress,
    backend: &Backend<EF, PF>,
) -> Result<(ContractAddress, Felt)>
where
    EF: ExecutorFactory,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: katana_provider::ProviderRO,
    <PF as ProviderFactory>::ProviderMut: katana_provider::ProviderRW,
{
    for (address, allocation) in backend.chain_spec.genesis().accounts() {
        if *address == sequencer_address {
            let private_key = match allocation {
                GenesisAccountAlloc::DevAccount(account) => account.private_key,
                _ => return Err(anyhow!("sequencer account has no private key")),
            };
            return Ok((*address, private_key));
        }
    }

    Err(anyhow!("sequencer key source requested but sequencer is not a prefunded account"))
}

#[cfg(feature = "vrf")]
struct DeploymentRequest {
    sender_address: ContractAddress,
    sender_private_key: Felt,
    target_address: ContractAddress,
    class_hash: Felt,
    constructor_calldata: Vec<Felt>,
    salt: Felt,
}

#[cfg(feature = "vrf")]
async fn ensure_deployed<EF, PF>(
    backend: &Backend<EF, PF>,
    block_producer: &BlockProducer<EF, PF>,
    pool: &TxPool,
    request: DeploymentRequest,
) -> Result<()>
where
    EF: ExecutorFactory,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: katana_provider::ProviderRO,
    <PF as ProviderFactory>::ProviderMut: katana_provider::ProviderRW,
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
    <PF as ProviderFactory>::Provider: katana_provider::ProviderRO,
    <PF as ProviderFactory>::ProviderMut: katana_provider::ProviderRW,
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

#[cfg(feature = "vrf")]
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
    <PF as ProviderFactory>::Provider: katana_provider::ProviderRO,
    <PF as ProviderFactory>::ProviderMut: katana_provider::ProviderRW,
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

#[cfg(feature = "vrf")]
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

#[cfg(feature = "vrf")]
fn sign_invoke_tx(
    chain_id: ChainId,
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

#[cfg(feature = "vrf")]
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

#[cfg(feature = "vrf")]
fn udc_calldata(class_hash: Felt, salt: Felt, constructor_calldata: Vec<Felt>) -> Vec<Felt> {
    let mut calldata = Vec::with_capacity(4 + constructor_calldata.len());
    calldata.push(class_hash);
    calldata.push(salt);
    calldata.push(Felt::ZERO);
    calldata.push(Felt::from(constructor_calldata.len()));
    calldata.extend(constructor_calldata);
    calldata
}

#[cfg(feature = "vrf")]
fn is_deployed<EF, PF>(backend: &Backend<EF, PF>, address: ContractAddress) -> Result<bool>
where
    EF: ExecutorFactory,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: katana_provider::ProviderRO,
    <PF as ProviderFactory>::ProviderMut: katana_provider::ProviderRW,
{
    let state = backend.storage.provider().latest()?;
    Ok(state.class_hash_of_contract(address)?.is_some())
}

#[cfg(feature = "vrf")]
async fn wait_for_contract<EF, PF>(
    backend: &Backend<EF, PF>,
    address: ContractAddress,
    timeout: Duration,
) -> Result<()>
where
    EF: ExecutorFactory,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: katana_provider::ProviderRO,
    <PF as ProviderFactory>::ProviderMut: katana_provider::ProviderRW,
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
    pub vrf: Option<VrfSidecarConfig<'a>>,
}

/// Configuration for starting the paymaster sidecar.
pub struct PaymasterStartConfig<'a> {
    pub options: &'a PaymasterOptions,
    pub port: u16,
    pub api_key: String,
    pub rpc_url: Url,
}

/// Configuration for the VRF sidecar.
#[cfg(feature = "vrf")]
pub struct VrfSidecarConfig<'a> {
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
            bin: paymaster_cfg.options.bin.clone(),
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
        vrf_child = Some(start_vrf_sidecar(vrf_cfg, vrf_bootstrap).await?);
    }

    #[cfg(feature = "vrf")]
    let processes = SidecarProcesses::new(paymaster_child, vrf_child);
    #[cfg(not(feature = "vrf"))]
    let processes = SidecarProcesses::new(paymaster_child);

    Ok(processes)
}

#[cfg(feature = "vrf")]
async fn start_vrf_sidecar(
    config: &VrfSidecarConfig<'_>,
    bootstrap: &VrfBootstrap,
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

#[cfg(feature = "vrf")]
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
#[cfg(feature = "paymaster")]
pub async fn bootstrap_and_start_sidecars<EF, PF>(
    paymaster_options: &PaymasterOptions,
    #[cfg(feature = "vrf")] vrf_options: &VrfOptions,
    backend: &Backend<EF, PF>,
    block_producer: &BlockProducer<EF, PF>,
    pool: &TxPool,
    rpc_addr: &SocketAddr,
    paymaster_sidecar: Option<&PaymasterSidecarInfo>,
    #[cfg(feature = "vrf")] vrf_sidecar: Option<&VrfSidecarInfo>,
    fee_enabled: bool,
    #[cfg(feature = "vrf")] sequencer_address: ContractAddress,
) -> Result<Option<SidecarProcesses>>
where
    EF: ExecutorFactory,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: katana_provider::ProviderRO,
    <PF as ProviderFactory>::ProviderMut: katana_provider::ProviderRW,
{
    // If no sidecars need to be started, return None
    #[cfg(feature = "vrf")]
    let has_sidecars = paymaster_sidecar.is_some() || vrf_sidecar.is_some();
    #[cfg(not(feature = "vrf"))]
    let has_sidecars = paymaster_sidecar.is_some();

    if !has_sidecars {
        return Ok(None);
    }

    // Build RPC URL for paymaster bootstrap
    let rpc_url = local_rpc_url(rpc_addr);

    // Build bootstrap config
    let bootstrap_config = BootstrapConfig {
        paymaster: paymaster_sidecar.map(|_| PaymasterBootstrapInput {
            rpc_url: rpc_url.clone(),
            prefunded_index: paymaster_options.prefunded_index,
        }),
        #[cfg(feature = "vrf")]
        vrf: vrf_sidecar.map(|_| {
            let key_source = match vrf_options.key_source {
                OptionsVrfKeySource::Prefunded => VrfKeySource::Prefunded,
                OptionsVrfKeySource::Sequencer => VrfKeySource::Sequencer,
            };
            VrfBootstrapConfig {
                key_source,
                prefunded_index: vrf_options.prefunded_index,
                sequencer_address,
            }
        }),
        fee_enabled,
    };

    // Bootstrap contracts
    let bootstrap = bootstrap_sidecars(&bootstrap_config, backend, block_producer, pool).await?;

    // Build sidecar start config
    let paymaster_config = paymaster_sidecar.map(|info| PaymasterStartConfig {
        options: paymaster_options,
        port: info.port,
        api_key: info.api_key.clone(),
        rpc_url: rpc_url.clone(),
    });

    #[cfg(feature = "vrf")]
    let vrf_config =
        vrf_sidecar.map(|info| VrfSidecarConfig { options: vrf_options, port: info.port });

    let start_config = SidecarStartConfig {
        paymaster: paymaster_config,
        #[cfg(feature = "vrf")]
        vrf: vrf_config,
    };

    // Start sidecar processes
    let processes = start_sidecars(&start_config, &bootstrap).await?;
    Ok(Some(processes))
}

#[cfg(test)]
mod tests {
    use katana_primitives::Felt;

    #[cfg(feature = "vrf")]
    use super::vrf_secret_key_from_account_key;

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
}
