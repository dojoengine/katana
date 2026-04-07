//! RPC executor for [`BootstrapPlan`].
//!
//! Mirrors the pattern used by `crates/cartridge/src/vrf/server/bootstrap.rs`:
//! a `SingleOwnerAccount` over a starknet-rs `JsonRpcClient`, with `declare_v3` for
//! class declarations and `ContractFactory::deploy_v3` for deployments. Each step is
//! followed by a polling wait so the next step observes the new state.

use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use katana_primitives::class::ClassHash;
use katana_primitives::{ContractAddress, Felt};
use katana_primitives::utils::get_contract_address;
use katana_rpc_types::RpcSierraContractClass;
use starknet::accounts::{Account, ConnectedAccount, ExecutionEncoding, SingleOwnerAccount};
use starknet::contract::ContractFactory;
use starknet::core::types::{BlockId, BlockTag, FlattenedSierraClass, StarknetError};
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider, ProviderError};
use starknet::signers::{LocalWallet, SigningKey};
use tokio::sync::mpsc::UnboundedSender;
use tokio::time::sleep;
use url::Url;

use crate::plan::{BootstrapPlan, DeclareStep, DeployStep};

/// Streaming progress events emitted by [`execute_with_progress`]. Consumers (the TUI)
/// receive these in order on a tokio mpsc channel as the executor walks through the plan.
#[derive(Debug, Clone)]
pub enum BootstrapEvent {
    DeclareStarted { idx: usize, name: String, class_hash: ClassHash },
    DeclareCompleted {
        idx: usize,
        name: String,
        class_hash: ClassHash,
        already_declared: bool,
    },
    DeployStarted { idx: usize, label: Option<String>, class_name: String },
    DeployCompleted {
        idx: usize,
        label: Option<String>,
        class_name: String,
        address: ContractAddress,
        tx_hash: Felt,
    },
    /// Terminal event on failure. The executor returns the error to its direct caller as
    /// well; this event exists so async consumers (the TUI) don't have to await the join
    /// handle to learn about it.
    Failed { error: String },
    /// Terminal event on success.
    Done { report: BootstrapReport },
}

/// Optional channel sink fed by [`execute_with_progress`].
pub type EventSink = UnboundedSender<BootstrapEvent>;

/// Default per-operation wait timeout for the on-chain visibility polls.
pub const STEP_TIMEOUT: Duration = Duration::from_secs(15);

/// What actually happened on-chain. Returned to the caller for pretty-printing.
#[derive(Debug, Clone, Default)]
pub struct BootstrapReport {
    pub declared: Vec<DeclaredClass>,
    pub deployed: Vec<DeployedContract>,
}

#[derive(Debug, Clone)]
pub struct DeclaredClass {
    pub name: String,
    pub class_hash: ClassHash,
    /// `true` if the declare was skipped because the class already existed.
    pub already_declared: bool,
}

#[derive(Debug, Clone)]
pub struct DeployedContract {
    pub label: Option<String>,
    pub class_name: String,
    pub address: ContractAddress,
    pub tx_hash: Felt,
}

/// Knobs the CLI surface lets the user pass through.
pub struct ExecutorConfig {
    pub rpc_url: Url,
    pub account_address: ContractAddress,
    pub private_key: Felt,
    /// If `true`, declares whose class is already on-chain are reported as a no-op
    /// instead of returning an error.
    pub skip_existing: bool,
}

/// Run the plan against a live katana node. Convenience wrapper around
/// [`execute_with_progress`] for callers that don't care about per-step events.
pub async fn execute(plan: &BootstrapPlan, cfg: &ExecutorConfig) -> Result<BootstrapReport> {
    execute_with_progress(plan, cfg, None).await
}

/// Run the plan against a live katana node, optionally streaming per-step progress
/// events to `sink`. The returned `Result<BootstrapReport>` is identical to what the
/// terminal `Done`/`Failed` event carries — both reporting paths exist so synchronous
/// callers don't have to wire a channel just to learn the outcome.
pub async fn execute_with_progress(
    plan: &BootstrapPlan,
    cfg: &ExecutorConfig,
    sink: Option<EventSink>,
) -> Result<BootstrapReport> {
    match execute_inner(plan, cfg, sink.as_ref()).await {
        Ok(report) => {
            if let Some(tx) = &sink {
                let _ = tx.send(BootstrapEvent::Done { report: report.clone() });
            }
            Ok(report)
        }
        Err(err) => {
            if let Some(tx) = &sink {
                let _ = tx.send(BootstrapEvent::Failed { error: format!("{err:#}") });
            }
            Err(err)
        }
    }
}

async fn execute_inner(
    plan: &BootstrapPlan,
    cfg: &ExecutorConfig,
    sink: Option<&EventSink>,
) -> Result<BootstrapReport> {
    let provider = JsonRpcClient::new(HttpTransport::new(cfg.rpc_url.clone()));
    let chain_id = provider
        .chain_id()
        .await
        .with_context(|| format!("failed to reach katana at {}", cfg.rpc_url))?;

    let signer = LocalWallet::from(SigningKey::from_secret_scalar(cfg.private_key));
    let account = SingleOwnerAccount::new(
        provider.clone(),
        signer,
        cfg.account_address.into(),
        chain_id,
        ExecutionEncoding::New,
    );

    // Fetch the starting nonce once and increment locally per submission. This avoids
    // a per-tx round trip and gives us deterministic, contiguous nonces — which is also
    // required by the sequencer when batching multiple txs from the same account before
    // a block is mined.
    let mut nonce = account
        .get_nonce()
        .await
        .with_context(|| format!("failed to fetch nonce for {}", cfg.account_address))?;

    let mut report = BootstrapReport::default();

    for (idx, step) in plan.declares.iter().enumerate() {
        if let Some(tx) = sink {
            let _ = tx.send(BootstrapEvent::DeclareStarted {
                idx,
                name: step.name.clone(),
                class_hash: step.class_hash,
            });
        }
        let outcome = run_declare(&account, step, nonce, cfg.skip_existing).await?;
        if !outcome.already_declared {
            nonce += Felt::ONE;
        }
        if let Some(tx) = sink {
            let _ = tx.send(BootstrapEvent::DeclareCompleted {
                idx,
                name: outcome.name.clone(),
                class_hash: outcome.class_hash,
                already_declared: outcome.already_declared,
            });
        }
        report.declared.push(outcome);
    }

    for (idx, step) in plan.deploys.iter().enumerate() {
        if let Some(tx) = sink {
            let _ = tx.send(BootstrapEvent::DeployStarted {
                idx,
                label: step.label.clone(),
                class_name: step.class_name.clone(),
            });
        }
        let outcome = run_deploy(&account, step, nonce).await?;
        nonce += Felt::ONE;
        if let Some(tx) = sink {
            let _ = tx.send(BootstrapEvent::DeployCompleted {
                idx,
                label: outcome.label.clone(),
                class_name: outcome.class_name.clone(),
                address: outcome.address,
                tx_hash: outcome.tx_hash,
            });
        }
        report.deployed.push(outcome);
    }

    Ok(report)
}

async fn run_declare(
    account: &SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>,
    step: &DeclareStep,
    nonce: Felt,
    skip_existing: bool,
) -> Result<DeclaredClass> {
    let provider = account_provider(account);

    if is_declared(provider, step.class_hash).await? {
        if skip_existing {
            return Ok(DeclaredClass {
                name: step.name.clone(),
                class_hash: step.class_hash,
                already_declared: true,
            });
        }
        return Err(anyhow!(
            "class `{}` ({:#x}) is already declared on-chain; pass --skip-existing to ignore",
            step.name,
            step.class_hash
        ));
    }

    let sierra = step
        .class
        .as_ref()
        .clone()
        .to_sierra()
        .ok_or_else(|| anyhow!("class `{}`: not a Sierra class", step.name))?;
    let rpc_class = RpcSierraContractClass::from(sierra);
    let flattened = FlattenedSierraClass::try_from(rpc_class)
        .with_context(|| format!("class `{}`: failed to flatten Sierra class", step.name))?;

    // Nonce is set explicitly so multiple txs from the same account can be batched before
    // a block is mined (otherwise starknet-rs would auto-fetch the same on-chain nonce for
    // every tx). Resource bounds are deliberately *not* set so estimate_fee runs against
    // the live node — this is what makes bootstrap chain-agnostic.
    let result = account
        .declare_v3(flattened.into(), step.casm_hash)
        .nonce(nonce)
        .send()
        .await
        .with_context(|| format!("class `{}`: declare_v3 failed", step.name))?;

    if result.class_hash != step.class_hash {
        return Err(anyhow!(
            "class `{}`: node returned class hash {:#x} but expected {:#x}",
            step.name,
            result.class_hash,
            step.class_hash
        ));
    }

    wait_for_class(provider, step.class_hash, STEP_TIMEOUT).await?;

    Ok(DeclaredClass {
        name: step.name.clone(),
        class_hash: step.class_hash,
        already_declared: false,
    })
}

async fn run_deploy(
    account: &SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>,
    step: &DeployStep,
    nonce: Felt,
) -> Result<DeployedContract> {
    let provider = account_provider(account);

    // Compute the deterministic deploy address up front so we can both display it and
    // poll for it after submission. UDC with `unique = false` uses ContractAddress::ZERO
    // as the deployer in the address derivation.
    let deployer_for_address: ContractAddress = if step.unique {
        account.address().into()
    } else {
        ContractAddress::ZERO
    };
    let address: ContractAddress = get_contract_address(
        step.salt,
        step.class_hash,
        &step.calldata,
        deployer_for_address,
    )
    .into();

    #[allow(deprecated)]
    let factory = ContractFactory::new(step.class_hash, &account);
    let result = factory
        .deploy_v3(step.calldata.clone(), step.salt, step.unique)
        .nonce(nonce)
        .send()
        .await
        .with_context(|| format!("contract `{}`: deploy_v3 failed", step.class_name))?;

    wait_for_contract(provider, address, STEP_TIMEOUT).await?;

    Ok(DeployedContract {
        label: step.label.clone(),
        class_name: step.class_name.clone(),
        address,
        tx_hash: result.transaction_hash,
    })
}

fn account_provider<'a>(
    account: &'a SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>,
) -> &'a JsonRpcClient<HttpTransport> {
    use starknet::accounts::ConnectedAccount;
    account.provider()
}

async fn is_declared(
    provider: &JsonRpcClient<HttpTransport>,
    class_hash: ClassHash,
) -> Result<bool> {
    match provider.get_class(BlockId::Tag(BlockTag::PreConfirmed), class_hash).await {
        Ok(_) => Ok(true),
        Err(ProviderError::StarknetError(StarknetError::ClassHashNotFound)) => Ok(false),
        Err(e) => Err(anyhow!("failed to check class declaration: {e}")),
    }
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

async fn wait_for_class(
    provider: &JsonRpcClient<HttpTransport>,
    class_hash: ClassHash,
    timeout: Duration,
) -> Result<()> {
    let start = Instant::now();
    loop {
        if is_declared(provider, class_hash).await? {
            return Ok(());
        }
        if start.elapsed() > timeout {
            return Err(anyhow!("class {class_hash:#x} not declared before timeout"));
        }
        sleep(Duration::from_millis(200)).await;
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
