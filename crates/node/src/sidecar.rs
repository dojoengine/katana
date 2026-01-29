//! Sidecar bootstrap utilities.
//!
//! This module contains the logic for bootstrapping paymaster and VRF services.
//! The actual process management (spawning sidecar processes) is handled by the CLI.

use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
#[cfg(feature = "vrf")]
use ark_ff::PrimeField;
use katana_core::backend::Backend;
use katana_core::service::block_producer::BlockProducer;
use katana_executor::ExecutorFactory;
use katana_genesis::allocation::GenesisAccountAlloc;
use katana_genesis::constant::{DEFAULT_STRK_FEE_TOKEN_ADDRESS, DEFAULT_UDC_ADDRESS};
use katana_pool::TxPool;
use katana_pool_api::TransactionPool;
use katana_primitives::chain::ChainId;
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
#[cfg(feature = "vrf")]
use stark_vrf::{generate_public_key, ScalarField};
use starknet::macros::selector;
use starknet::signers::{LocalWallet, Signer, SigningKey};
use tokio::time::sleep;

use crate::config::paymaster::PaymasterConfig;
#[cfg(feature = "vrf")]
use crate::config::paymaster::{VrfConfig, VrfKeySource};
use crate::config::Config;

const FORWARDER_SALT: u64 = 0x12345;
#[cfg(feature = "vrf")]
const VRF_ACCOUNT_SALT: u64 = 0x54321;
#[cfg(feature = "vrf")]
const VRF_CONSUMER_SALT: u64 = 0x67890;
const BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(10);

/// Bootstrap data for the paymaster service.
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

/// Bootstrap data for the VRF service.
#[cfg(feature = "vrf")]
#[derive(Debug, Clone)]
pub struct VrfBootstrap {
    pub secret_key: u64,
}

/// Result of bootstrapping sidecars.
#[derive(Debug, Default)]
pub struct BootstrapResult {
    pub paymaster: Option<PaymasterBootstrap>,
    #[cfg(feature = "vrf")]
    pub vrf: Option<VrfBootstrap>,
}

/// Bootstrap sidecars by deploying necessary contracts and preparing configuration.
///
/// This must be called after the node is built but before sidecars are started.
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
        let bootstrap = bootstrap_paymaster(paymaster_cfg, backend, block_producer, pool).await?;
        result.paymaster = Some(bootstrap);
    }

    #[cfg(feature = "vrf")]
    if let Some(vrf_cfg) = config.vrf.as_ref() {
        let bootstrap = bootstrap_vrf(vrf_cfg, config, backend, block_producer, pool).await?;
        result.vrf = Some(bootstrap);
    }

    Ok(result)
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

/// Format a Felt as a hex string with 0x prefix.
pub fn format_felt(value: Felt) -> String {
    format!("{value:#x}")
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
