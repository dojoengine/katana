#![cfg(feature = "cartridge")]

use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use ark_ff::PrimeField;
use katana_core::backend::Backend;
use katana_core::service::block_producer::BlockProducer;
use katana_executor::ExecutorFactory;
use katana_genesis::allocation::GenesisAccountAlloc;
use katana_genesis::constant::{DEFAULT_STRK_FEE_TOKEN_ADDRESS, DEFAULT_UDC_ADDRESS};
use katana_pool::TxPool;
use katana_pool_api::TransactionPool;
use katana_primitives::chain::{ChainId, NamedChainId};
use katana_primitives::da::DataAvailabilityMode;
use katana_primitives::fee::{AllResourceBoundsMapping, ResourceBoundsMapping};
use katana_primitives::transaction::{ExecutableTx, ExecutableTxWithHash, InvokeTx, InvokeTxV3};
use katana_primitives::utils::{get_contract_address, split_u256};
use katana_primitives::{ContractAddress, Felt, U256};
use katana_provider::api::state::{StateFactoryProvider, StateProvider};
use katana_provider::ProviderFactory;
use katana_rpc_types::FunctionCall;
use stark_vrf::{generate_public_key, ScalarField};
use starknet::macros::selector;
use starknet::signers::{LocalWallet, Signer, SigningKey};
use tokio::process::{Child, Command};
use tokio::time::sleep;
use tracing::{debug, info, warn};
use url::Url;

use crate::config::paymaster::{PaymasterConfig, ServiceMode, VrfConfig, VrfKeySource};
use crate::config::Config;

const FORWARDER_SALT: u64 = 0x12345;
const VRF_ACCOUNT_SALT: u64 = 0x54321;
const VRF_CONSUMER_SALT: u64 = 0x67890;
const BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub struct SidecarProcesses {
    paymaster: Option<Child>,
    vrf: Option<Child>,
}

impl SidecarProcesses {
    pub fn new(paymaster: Option<Child>, vrf: Option<Child>) -> Self {
        Self { paymaster, vrf }
    }
}

impl Drop for SidecarProcesses {
    fn drop(&mut self) {
        if let Some(mut child) = self.paymaster.take() {
            let _ = child.start_kill();
        }
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
    pub chain_id: String,
}

#[derive(Debug, Clone)]
pub struct VrfBootstrap {
    pub account_address: ContractAddress,
    pub account_private_key: Felt,
    pub secret_key: Felt,
}

#[derive(Debug, Default)]
pub struct BootstrapResult {
    pub paymaster: Option<PaymasterBootstrap>,
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
    let mut vrf_child = None;

    if let (Some(paymaster_cfg), Some(paymaster_bootstrap)) =
        (config.paymaster.as_ref(), bootstrap.paymaster.as_ref())
    {
        if paymaster_cfg.mode == ServiceMode::Sidecar {
            paymaster_child = Some(start_paymaster_sidecar(
                paymaster_cfg,
                paymaster_bootstrap,
                rpc_addr,
            )
            .await?);
        }
    }

    if let (Some(vrf_cfg), Some(vrf_bootstrap)) =
        (config.vrf.as_ref(), bootstrap.vrf.as_ref())
    {
        if vrf_cfg.mode == ServiceMode::Sidecar {
            vrf_child = Some(start_vrf_sidecar(vrf_cfg, vrf_bootstrap).await?);
        }
    }

    Ok(SidecarProcesses::new(paymaster_child, vrf_child))
}

async fn start_paymaster_sidecar(
    config: &PaymasterConfig,
    bootstrap: &PaymasterBootstrap,
    rpc_addr: &std::net::SocketAddr,
) -> Result<Child> {
    let bin = config
        .sidecar_bin
        .clone()
        .unwrap_or_else(|| "katana-paymaster".into());
    let rpc_url = local_rpc_url(rpc_addr);

    let mut command = Command::new(bin);
    command
        .arg("--port")
        .arg(config.sidecar_port.to_string())
        .arg("--rpc-url")
        .arg(rpc_url.as_str())
        .arg("--chain-id")
        .arg(&bootstrap.chain_id)
        .arg("--forwarder")
        .arg(bootstrap.forwarder_address.to_string())
        .arg("--account-address")
        .arg(bootstrap.relayer_address.to_string())
        .arg("--account-private-key")
        .arg(format!("{:#x}", bootstrap.relayer_private_key))
        .arg("--supported-token")
        .arg("eth")
        .arg("--supported-token")
        .arg("strk")
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);

    if let Some(api_key) = &config.api_key {
        command.arg("--api-key").arg(api_key);
    }

    let child = command.spawn().context("failed to spawn paymaster sidecar")?;

    wait_for_http_ok(
        &format!("{}/health", config.url),
        "paymaster health",
        BOOTSTRAP_TIMEOUT,
    )
    .await?;

    Ok(child)
}

async fn start_vrf_sidecar(config: &VrfConfig, bootstrap: &VrfBootstrap) -> Result<Child> {
    let bin = config
        .sidecar_bin
        .clone()
        .unwrap_or_else(|| "katana-vrf".into());

    let mut command = Command::new(bin);
    command
        .arg("--port")
        .arg(config.sidecar_port.to_string())
        .arg("--secret-key")
        .arg(format!("{:#x}", bootstrap.secret_key))
        .arg("--account-address")
        .arg(bootstrap.account_address.to_string())
        .arg("--account-private-key")
        .arg(format!("{:#x}", bootstrap.account_private_key))
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);

    let child = command.spawn().context("failed to spawn vrf sidecar")?;

    wait_for_http_ok(&format!("{}/info", config.url), "vrf info", BOOTSTRAP_TIMEOUT).await?;

    Ok(child)
}

fn local_rpc_url(addr: &std::net::SocketAddr) -> Url {
    let host = match addr.ip() {
        std::net::IpAddr::V4(ip) if ip.is_unspecified() => std::net::IpAddr::V4([127, 0, 0, 1].into()),
        std::net::IpAddr::V6(ip) if ip.is_unspecified() => std::net::IpAddr::V4([127, 0, 0, 1].into()),
        ip => ip,
    };

    Url::parse(&format!("http://{}:{}", host, addr.port())).expect("valid rpc url")
}

fn paymaster_chain_id(chain_id: ChainId) -> Result<String> {
    match chain_id {
        ChainId::Named(NamedChainId::Sepolia) => Ok(NamedChainId::Sepolia.name().to_string()),
        ChainId::Named(NamedChainId::Mainnet) => Ok("SN_MAINNET".to_string()),
        ChainId::Named(other) => Err(anyhow!(
            "paymaster sidecar only supports SN_MAIN or SN_SEPOLIA chain ids, got {other}"
        )),
        ChainId::Id(id) => Err(anyhow!(
            "paymaster sidecar requires SN_MAIN or SN_SEPOLIA chain id, got {id:#x}"
        )),
    }
}

fn scalar_from_felt(value: Felt) -> ScalarField {
    let bytes = value.to_bytes_be();
    ScalarField::from_be_bytes_mod_order(&bytes)
}

fn felt_from_field<T: std::fmt::Display>(value: T) -> Result<Felt> {
    let decimal = value.to_string();
    Felt::from_dec_str(&decimal).map_err(|err| anyhow!("invalid field value: {err}"))
}

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

    let forwarder_class_hash = avnu_forwarder_class_hash()?;
    let forwarder_address = get_contract_address(
        Felt::from(FORWARDER_SALT),
        forwarder_class_hash,
        &[relayer_address.into(), relayer_address.into()],
        DEFAULT_UDC_ADDRESS.into(),
    )
    .into();

    ensure_deployed(
        backend,
        block_producer,
        pool,
        relayer_address,
        relayer_private_key,
        forwarder_address,
        forwarder_class_hash,
        vec![relayer_address.into(), relayer_address.into()],
        Felt::from(FORWARDER_SALT),
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

    let chain_id = paymaster_chain_id(backend.chain_spec.id())?;

    Ok(PaymasterBootstrap {
        forwarder_address,
        relayer_address,
        relayer_private_key,
        chain_id,
    })
}

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
    let (account_address, account_private_key) = match config.key_source {
        VrfKeySource::Prefunded => prefunded_account(backend, config.prefunded_index)?,
        VrfKeySource::Sequencer => sequencer_account(node_config, backend)?,
    };

    let secret_key = account_private_key;
    let public_key = generate_public_key(scalar_from_felt(secret_key));
    let public_key_x = felt_from_field(public_key.x)?;
    let public_key_y = felt_from_field(public_key.y)?;

    let account_public_key = SigningKey::from_secret_scalar(account_private_key).verifying_key().scalar();

    let vrf_account_class_hash = vrf_account_class_hash()?;
    let vrf_account_address = get_contract_address(
        Felt::from(VRF_ACCOUNT_SALT),
        vrf_account_class_hash,
        &[account_public_key],
        DEFAULT_UDC_ADDRESS.into(),
    )
    .into();

    ensure_deployed(
        backend,
        block_producer,
        pool,
        account_address,
        account_private_key,
        vrf_account_address,
        vrf_account_class_hash,
        vec![account_public_key],
        Felt::from(VRF_ACCOUNT_SALT),
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
        calldata: vec![public_key_x, public_key_y],
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
        account_address,
        account_private_key,
        vrf_consumer_address,
        vrf_consumer_class_hash,
        vec![vrf_account_address.into()],
        Felt::from(VRF_CONSUMER_SALT),
    )
    .await?;

    Ok(VrfBootstrap {
        account_address: vrf_account_address,
        account_private_key,
        secret_key,
    })
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
    let mut accounts = backend.chain_spec.genesis().accounts();

    while let Some((address, allocation)) = accounts.next() {
        if *address == sequencer {
            let private_key = match allocation {
                GenesisAccountAlloc::DevAccount(account) => account.private_key,
                _ => return Err(anyhow!("sequencer account has no private key")),
            };
            return Ok((*address, private_key));
        }
    }

    Err(anyhow!(
        "sequencer key source requested but sequencer is not a prefunded account"
    ))
}

async fn ensure_deployed<EF, PF>(
    backend: &Backend<EF, PF>,
    block_producer: &BlockProducer<EF, PF>,
    pool: &TxPool,
    sender_address: ContractAddress,
    sender_private_key: Felt,
    target_address: ContractAddress,
    class_hash: Felt,
    constructor_calldata: Vec<Felt>,
    salt: Felt,
) -> Result<()>
where
    EF: ExecutorFactory,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: katana_core::backend::storage::ProviderRO,
    <PF as ProviderFactory>::ProviderMut: katana_core::backend::storage::ProviderRW,
{
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

    let tx = sign_invoke_tx(
        backend.chain_spec.id(),
        sender_address,
        sender_private_key,
        nonce,
        calls,
    )?;

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
    let class = katana_primitives::utils::class::parse_sierra_class(
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../controller/classes/avnu_Forwarder.contract_class.json"
        )),
    )?;
    class.class_hash().context("failed to compute forwarder class hash")
}

fn vrf_account_class_hash() -> Result<Felt> {
    let class = katana_primitives::utils::class::parse_sierra_class(
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../controller/classes/cartridge_vrf_VrfAccount.contract_class.json"
        )),
    )?;
    class.class_hash().context("failed to compute vrf account class hash")
}

fn vrf_consumer_class_hash() -> Result<Felt> {
    let class = katana_primitives::utils::class::parse_sierra_class(
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../controller/classes/cartridge_vrf_VrfConsumer.contract_class.json"
        )),
    )?;
    class.class_hash().context("failed to compute vrf consumer class hash")
}
