//! RPC executor for [`BootstrapPlan`].
//!
//! Uses two parallel HTTP clients pointing at the same katana node:
//!
//! - a `starknet-rs` `JsonRpcClient` wrapped in a `SingleOwnerAccount`, for the `declare_v3` /
//!   `deploy_v3` submission flow (signing + estimate_fee + send);
//! - a `katana-starknet` `StarknetRpcClient`, for [`katana_utils::TxWaiter`] — our project-standard
//!   poll-and-wait utility, which surfaces revert reasons and shares timeout/interval defaults with
//!   the rest of the codebase.
//!
//! Each submitted tx is awaited via `TxWaiter` on its returned hash before the
//! executor moves on to the next step.

use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use katana_primitives::class::ClassHash;
use katana_primitives::utils::get_contract_address;
use katana_primitives::{ContractAddress, Felt};
use katana_rpc_types::RpcSierraContractClass;
use katana_starknet::rpc::StarknetRpcClient;
use katana_utils::{TxWaiter, TxWaitingError};
use starknet::accounts::{Account, ConnectedAccount, ExecutionEncoding, SingleOwnerAccount};
use starknet::contract::ContractFactory;
use starknet::core::types::{BlockId, BlockTag, FlattenedSierraClass, StarknetError};
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider, ProviderError};
use starknet::signers::{LocalWallet, SigningKey};
use tokio::sync::mpsc::UnboundedSender;
use url::Url;

use crate::plan::{BootstrapPlan, DeclareStep, DeployStep};

/// Streaming progress events emitted by [`execute_with_progress`]. Consumers (the TUI)
/// receive these in order on a tokio mpsc channel as the executor walks through the plan.
#[derive(Debug, Clone)]
pub enum BootstrapEvent {
    DeclareStarted {
        idx: usize,
        name: String,
        class_hash: ClassHash,
    },
    DeclareCompleted {
        idx: usize,
        name: String,
        class_hash: ClassHash,
        /// `true` if the class was already on-chain and the declare was skipped.
        already_declared: bool,
    },
    DeployStarted {
        idx: usize,
        label: Option<String>,
        class_name: String,
    },
    DeployCompleted {
        idx: usize,
        label: Option<String>,
        class_name: String,
        address: ContractAddress,
        /// Tx hash of the submitted invoke. `None` when the deploy was skipped because
        /// a contract was already at the deterministic address.
        tx_hash: Option<Felt>,
        /// `true` if a contract was already at the target address and the deploy was
        /// skipped.
        already_deployed: bool,
    },
    /// Terminal event on failure. The executor returns the error to its direct caller as
    /// well; this event exists so async consumers (the TUI) don't have to await the join
    /// handle to learn about it.
    Failed {
        error: String,
    },
    /// Terminal event on success.
    Done {
        report: BootstrapReport,
    },
}

/// Optional channel sink fed by [`execute_with_progress`].
pub type EventSink = UnboundedSender<BootstrapEvent>;

/// Per-tx wait timeout passed to [`TxWaiter`]. Defaults are 30s in `katana-utils`,
/// which is fine for a real chain; bootstrap is dev-leaning so we shorten it.
const STEP_TIMEOUT: Duration = Duration::from_secs(30);
/// How often [`TxWaiter`] re-polls. Default is 2.5s; for in-process katana we want
/// snappier feedback in the TUI, so we tighten it.
const STEP_POLL_INTERVAL_MS: u64 = 250;

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
    /// `None` when the deploy was skipped because a contract was already at the
    /// deterministic address. `Some(hash)` for genuinely-submitted deploys.
    pub tx_hash: Option<Felt>,
    /// `true` if a contract was already at the target address and the deploy was
    /// skipped (mirrors [`DeclaredClass::already_declared`]).
    pub already_deployed: bool,
}

/// Knobs the CLI surface lets the user pass through.
#[derive(Debug)]
pub struct ExecutorConfig {
    pub rpc_url: Url,
    pub account_address: ContractAddress,
    pub private_key: Felt,
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

    // Second client, same URL: TxWaiter expects a `katana_starknet::StarknetRpcClient`
    // (which goes through jsonrpsee + the project's RPC trait), not a starknet-rs
    // `JsonRpcClient`. Two clients is cheap — they share the underlying HTTP socket
    // pool inside reqwest/hyper.
    let starknet_client = StarknetRpcClient::new(cfg.rpc_url.clone());

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
        let outcome = run_declare(&account, &starknet_client, step, nonce).await?;
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
        let outcome = run_deploy(&account, &starknet_client, step, nonce).await?;
        if !outcome.already_deployed {
            nonce += Felt::ONE;
        }
        if let Some(tx) = sink {
            let _ = tx.send(BootstrapEvent::DeployCompleted {
                idx,
                label: outcome.label.clone(),
                class_name: outcome.class_name.clone(),
                address: outcome.address,
                tx_hash: outcome.tx_hash,
                already_deployed: outcome.already_deployed,
            });
        }
        report.deployed.push(outcome);
    }

    Ok(report)
}

async fn run_declare(
    account: &SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>,
    starknet_client: &StarknetRpcClient,
    step: &DeclareStep,
    nonce: Felt,
) -> Result<DeclaredClass> {
    // Pre-check: if the class is already on chain, skip cleanly. Bootstrap is
    // canonically idempotent — re-running the same plan against the same chain
    // should converge on the desired state without errors.
    if is_declared(account_provider(account), step.class_hash).await? {
        return Ok(DeclaredClass {
            name: step.name.clone(),
            class_hash: step.class_hash,
            already_declared: true,
        });
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

    wait_for_tx(starknet_client, result.transaction_hash)
        .await
        .with_context(|| format!("class `{}`: waiting for declare tx", step.name))?;

    Ok(DeclaredClass {
        name: step.name.clone(),
        class_hash: step.class_hash,
        already_declared: false,
    })
}

async fn run_deploy(
    account: &SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>,
    starknet_client: &StarknetRpcClient,
    step: &DeployStep,
    nonce: Felt,
) -> Result<DeployedContract> {
    // Compute the deterministic deploy address up front. Used both for the precheck
    // below (skip if a contract already lives at that address) and for the
    // BootstrapEvent::DeployCompleted payload.
    let address = compute_deploy_address(step, account.address().into());

    // Pre-check: if a contract is already at the deterministic address, skip the
    // deploy (mirrors the declare-side precheck — bootstrap is canonically
    // idempotent).
    if is_deployed(account_provider(account), address).await? {
        return Ok(DeployedContract {
            label: step.label.clone(),
            class_name: step.class_name.clone(),
            address,
            tx_hash: None,
            already_deployed: true,
        });
    }

    #[allow(deprecated)]
    let factory = ContractFactory::new(step.class_hash, &account);
    let result = factory
        .deploy_v3(step.calldata.clone(), step.salt, step.unique)
        .nonce(nonce)
        .send()
        .await
        .with_context(|| format!("contract `{}`: deploy_v3 failed", step.class_name))?;

    wait_for_tx(starknet_client, result.transaction_hash)
        .await
        .with_context(|| format!("contract `{}`: waiting for deploy tx", step.class_name))?;

    Ok(DeployedContract {
        label: step.label.clone(),
        class_name: step.class_name.clone(),
        address,
        tx_hash: Some(result.transaction_hash),
        already_deployed: false,
    })
}

fn account_provider(
    account: &SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>,
) -> &JsonRpcClient<HttpTransport> {
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

// -----------------------------------------------------------------------------
// Public idempotency helpers — shared with the TUI refresh path
// -----------------------------------------------------------------------------

/// Compute the deterministic deploy address for a [`DeployStep`] against a given
/// signer account. UDC with `unique = false` uses [`ContractAddress::ZERO`] as the
/// deployer in the derivation; `unique = true` uses the signer's address. The TUI
/// refresh path needs this to probe whether a contract is already on a different
/// node without committing to the full submission flow.
pub fn compute_deploy_address(
    step: &DeployStep,
    account_address: ContractAddress,
) -> ContractAddress {
    let deployer_for_address: ContractAddress =
        if step.unique { account_address } else { ContractAddress::ZERO };
    get_contract_address(step.salt, step.class_hash, &step.calldata, deployer_for_address).into()
}

/// Public idempotency probe: is this class hash already on-chain at the given RPC?
/// Wraps the same precheck used by [`run_declare`]. The TUI calls this from the
/// Settings-change refresh task to re-evaluate previously-executed items against a
/// possibly-different node without re-submitting anything.
pub async fn check_already_declared(rpc_url: &Url, class_hash: ClassHash) -> Result<bool> {
    let provider = JsonRpcClient::new(HttpTransport::new(rpc_url.clone()));
    is_declared(&provider, class_hash).await
}

/// Public idempotency probe: is a contract already at `address` on-chain at the
/// given RPC? Mirrors [`check_already_declared`] for deploys. Callers derive
/// `address` via [`compute_deploy_address`] when probing a [`DeployStep`].
pub async fn check_already_deployed(rpc_url: &Url, address: ContractAddress) -> Result<bool> {
    let provider = JsonRpcClient::new(HttpTransport::new(rpc_url.clone()));
    is_deployed(&provider, address).await
}

/// Block on a single tx via [`TxWaiter`] until it's included and successful. Maps the
/// project's `TxWaitingError` onto a flat `anyhow::Error` so callers can compose with
/// the rest of the executor's error reporting.
async fn wait_for_tx(client: &StarknetRpcClient, tx_hash: Felt) -> Result<()> {
    TxWaiter::new(tx_hash, client)
        .with_timeout(STEP_TIMEOUT)
        .with_interval(STEP_POLL_INTERVAL_MS)
        .await
        .map(|_| ())
        .map_err(|err| match err {
            TxWaitingError::Timeout => anyhow!("timed out after {:?}", STEP_TIMEOUT),
            TxWaitingError::TransactionReverted(reason) => {
                anyhow!("transaction reverted: {reason}")
            }
            TxWaitingError::Client(e) => anyhow!("rpc error: {e}"),
        })
}
