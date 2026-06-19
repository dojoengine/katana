//! L2 settlement contract deployment.
//!
//! Declares and deploys the two contracts the TEE settlement path needs on the
//! settlement chain, using the in-tree class artifacts embedded in
//! `katana-contracts` (rebuilt from the `cartridge-gg/piltover` submodule):
//!
//! 1. `piltover::MockAmdTeeRegistry` — a permissive `IAMDTeeRegistry` mock that round-trips the SP1
//!    journal without verification.
//! 2. `piltover::AppchainCoreContract` — the Piltover core contract.
//!
//! After deployment, Piltover is configured for TEE settlement with a
//! `set_program_info(KatanaTee variant)` + `set_facts_registry` multicall. The
//! `katana_tee_config_hash` written on-chain matches what L3's attestation
//! builder computes, so Piltover's `validate_input` environment-binding
//! assertions hold at settlement time.

use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use katana_chain_spec::tee::compute_katana_tee_config_hash;
use katana_contracts::piltover::{AppchainCoreContract, MockAmdTeeRegistry};
use katana_genesis::constant::DEFAULT_STRK_FEE_TOKEN_ADDRESS;
use katana_primitives::class::ContractClass;
use katana_rpc_types::class::Class;
use starknet::accounts::{Account, ConnectedAccount, ExecutionEncoding, SingleOwnerAccount};
use starknet::contract::{ContractFactory, UdcSelector};
use starknet::core::types::{BlockId, BlockTag, Call, FlattenedSierraClass, StarknetError};
use starknet::macros::{selector, short_string};
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider, ProviderError};
use starknet::signers::{LocalWallet, SigningKey};
use starknet_types_core::felt::Felt;

use crate::nodes::L2InProcess;

const PILTOVER_SALT: Felt = Felt::from_hex_unchecked("0x9117f0");
const TEE_REGISTRY_SALT: Felt = Felt::from_hex_unchecked("0x7ee");

/// Cairo enum variant index for `ProgramInfo::KatanaTee`. `StarknetOs` is index 0.
const KATANA_TEE_VARIANT_INDEX: Felt = Felt::ONE;

/// L3 chain id felt. Mirrors `nodes::spawn_l3`'s `ChainId::parse("KATANA")`.
/// Used to compute the `katana_tee_config_hash` that L3's attestations bind
/// into `report_data` and that on-chain Piltover asserts against.
const L3_CHAIN_ID: Felt = short_string!("KATANA");

type L2Account = SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>;

/// Runs the full L2 bootstrap sequence.
pub async fn bootstrap_l2(l2: &L2InProcess) -> Result<BootstrapResult> {
    // Pull the prefunded account from the dev genesis.
    let (account_address, account_private_key) = l2.prefunded_account_keys();

    let provider = l2.provider();
    let l2_chain_id = provider.chain_id().await.context("fetch L2 chain id")?;

    let signer = LocalWallet::from_signing_key(SigningKey::from_secret_scalar(account_private_key));
    let mut account = SingleOwnerAccount::new(
        provider,
        signer,
        account_address,
        l2_chain_id,
        ExecutionEncoding::New,
    );
    // Otherwise all the fee estimates after a transaction would run against stale state.
    account.set_block_id(BlockId::Tag(BlockTag::PreConfirmed));

    println!("Declaring + deploying mock TEE registry on L2");
    declare_class(
        &account,
        MockAmdTeeRegistry::HASH,
        (*MockAmdTeeRegistry::CLASS).clone(),
        MockAmdTeeRegistry::CASM_HASH,
    )
    .await?;
    let tee_registry_address =
        deploy_contract(&account, MockAmdTeeRegistry::HASH, vec![], TEE_REGISTRY_SALT).await?;
    println!("  tee_registry_address={}", hex(&tee_registry_address));

    println!("Declaring + deploying Piltover core contract on L2");
    declare_class(
        &account,
        AppchainCoreContract::HASH,
        (*AppchainCoreContract::CLASS).clone(),
        AppchainCoreContract::CASM_HASH,
    )
    .await?;
    // appchain::constructor(owner, state_root, block_number, block_hash). `block_number` must be
    // the Felt::MAX sentinel for the "nothing settled yet" genesis state.
    let constructor_calldata = vec![account.address(), Felt::ZERO, Felt::MAX, Felt::ZERO];
    let piltover_address =
        deploy_contract(&account, AppchainCoreContract::HASH, constructor_calldata, PILTOVER_SALT)
            .await?;
    println!("  piltover_address={}", hex(&piltover_address));

    println!("Configuring Piltover for KatanaTee settlement + mock TEE registry as fact_registry");
    configure_piltover_for_tee(&account, piltover_address, tee_registry_address).await?;

    Ok(BootstrapResult {
        piltover_address,
        tee_registry_address,
        account_address,
        account_private_key,
    })
}

/// Declares the given class if it is not already known to the chain.
async fn declare_class(
    account: &L2Account,
    class_hash: Felt,
    class: ContractClass,
    casm_hash: Felt,
) -> Result<()> {
    match account.provider().get_class(BlockId::Tag(BlockTag::PreConfirmed), class_hash).await {
        // Already declared.
        Ok(..) => return Ok(()),
        Err(ProviderError::StarknetError(StarknetError::ClassHashNotFound)) => {}
        Err(e) => return Err(e).context("check class declaration status"),
    }

    let rpc_class = flatten_class(class)?;
    let res = account
        .declare_v3(rpc_class.into(), casm_hash)
        .send()
        .await
        .with_context(|| format!("declare class {class_hash:#x}"))?;

    wait_for_tx(account.provider(), res.transaction_hash).await
}

/// Deploys an instance of the given class through the UDC and returns its address.
async fn deploy_contract(
    account: &L2Account,
    class_hash: Felt,
    constructor_calldata: Vec<Felt>,
    salt: Felt,
) -> Result<Felt> {
    let factory = ContractFactory::new_with_udc(class_hash, account, UdcSelector::Legacy);
    let request = factory.deploy_v3(constructor_calldata, salt, false);
    let address = request.deployed_address();

    let res = request
        .send()
        .await
        .with_context(|| format!("deploy contract of class {class_hash:#x}"))?;
    wait_for_tx(account.provider(), res.transaction_hash).await?;

    Ok(address)
}

fn flatten_class(class: ContractClass) -> Result<FlattenedSierraClass> {
    let rpc_class = Class::try_from(class).context("convert class to RPC representation")?;
    let Class::Sierra(class) = rpc_class else { unreachable!("unexpected legacy class") };
    Ok(class.try_into()?)
}

/// Configures the freshly-deployed Piltover for TEE settlement: sets the `KatanaTee`
/// `ProgramInfo` variant whose `katana_tee_config_hash` matches what L3's attestations will
/// carry, and points `fact_registry` at the permissive mock TEE registry.
///
/// Bundles `set_program_info` + `set_facts_registry` in one multicall.
async fn configure_piltover_for_tee(
    account: &L2Account,
    piltover_address: Felt,
    tee_registry_address: Felt,
) -> Result<()> {
    let katana_tee_config_hash =
        compute_katana_tee_config_hash(L3_CHAIN_ID, DEFAULT_STRK_FEE_TOKEN_ADDRESS.into());

    // ProgramInfo::KatanaTee(KatanaTeeProgramInfo { katana_tee_config_hash })
    // serializes as [variant_index=1, katana_tee_config_hash].
    let set_program_info = Call {
        to: piltover_address,
        selector: selector!("set_program_info"),
        calldata: vec![KATANA_TEE_VARIANT_INDEX, katana_tee_config_hash],
    };
    let set_facts_registry = Call {
        to: piltover_address,
        selector: selector!("set_facts_registry"),
        calldata: vec![tee_registry_address],
    };

    let result = account
        .execute_v3(vec![set_program_info, set_facts_registry])
        .send()
        .await
        .context("set_program_info + set_facts_registry multicall failed")?;

    wait_for_tx(account.provider(), result.transaction_hash).await
}

async fn wait_for_tx(provider: &JsonRpcClient<HttpTransport>, tx_hash: Felt) -> Result<()> {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        match provider.get_transaction_receipt(tx_hash).await {
            Ok(_) => return Ok(()),
            Err(_) if std::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            Err(e) => return Err(anyhow!("tx {tx_hash:#x} not accepted: {e}")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct BootstrapResult {
    /// The address of the deployed Piltover core contract on the settlement layer.
    pub piltover_address: Felt,
    /// The address of the deployed TEE regsitry contract on the settlement layer.
    pub tee_registry_address: Felt,
    /// The address of the account used for the bootstrapping.
    pub account_address: Felt,
    /// The private key of the account used for the bootstrapping.
    pub account_private_key: Felt,
}

fn hex(felt: &Felt) -> String {
    format!("0x{:x}", felt)
}
