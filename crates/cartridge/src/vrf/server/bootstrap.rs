//! VRF bootstrap and account derivation.
//!
//! This module handles:
//! - Deriving VRF accounts and keys from a fixed VRF secret
//! - Deploying VRF account and consumer contracts via RPC
//! - Setting up VRF public keys on deployed accounts
//!
//! This module uses the starknet crate's account abstraction for transaction handling.

use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use ark_ff::PrimeField;
use katana_contracts::vrf::{CartridgeVrfAccount, CartridgeVrfConsumer};
use katana_genesis::constant::DEFAULT_STRK_FEE_TOKEN_ADDRESS;
use katana_primitives::chain::ChainId;
use katana_primitives::class::ClassHash;
use katana_primitives::utils::get_contract_address;
use katana_primitives::{ContractAddress, Felt};
use katana_rpc_types::RpcSierraContractClass;
use stark_vrf::{generate_public_key, ScalarField};
use starknet::accounts::{Account, ConnectedAccount, ExecutionEncoding, SingleOwnerAccount};
use starknet::contract::ContractFactory;
use starknet::core::types::{BlockId, BlockTag, Call, FlattenedSierraClass, StarknetError};
use starknet::macros::selector;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider, ProviderError};
use starknet::signers::{LocalWallet, SigningKey};
use tokio::time::sleep;
use url::Url;

/// Salt used for deploying VRF accounts via UDC.
pub const VRF_ACCOUNT_SALT: u64 = 0x54321;
/// Salt used for deploying VRF consumer contracts via UDC.
pub const VRF_CONSUMER_SALT: u64 = 0x67890;
/// Hardcoded VRF secret key used to derive VRF account credentials.
pub const VRF_HARDCODED_SECRET_KEY: u64 = 0x111;
/// Default timeout for bootstrap operations.
pub const BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(10);

// ============================================================================
// Bootstrap Configuration Types
// ============================================================================

/// Bootstrap data for the VRF service.
#[derive(Debug, Clone)]
pub struct VrfBootstrap {
    pub secret_key: u64,
}

/// Bootstrap configuration for the VRF service.
/// These are the input parameters needed to perform bootstrap.
#[derive(Debug, Clone)]
pub struct VrfBootstrapConfig {
    /// RPC URL of the katana node.
    pub rpc_url: Url,

    /// Source account address (used to deploy contracts and fund VRF account).
    pub bootstrapper_account: ContractAddress,
    /// Source account private key.
    pub bootstrapper_account_private_key: Felt,
}

/// Result of VRF bootstrap operations.
#[derive(Debug, Clone)]
pub struct VrfBootstrapResult {
    /// The VRF secret key used by the VRF sidecar.
    pub secret_key: u64,
    pub vrf_account_address: ContractAddress,
    pub vrf_account_private_key: Felt,
    pub vrf_consumer_address: ContractAddress,
    pub chain_id: ChainId,
}

/// Derived VRF account information.
#[derive(Debug, Clone)]
pub struct VrfAccountCredentials {
    pub private_key: Felt,
    pub account_address: ContractAddress,
    pub vrf_public_key_x: Felt,
    pub vrf_public_key_y: Felt,
    pub secret_key: u64,
}

// ============================================================================
// Bootstrap Functions
// ============================================================================

/// Derive VRF accounts from the hardcoded VRF secret key.
///
/// This computes the deterministic VRF account address and VRF key pair
/// from a fixed VRF private key.
pub fn get_vrf_account() -> Result<VrfAccountCredentials> {
    let secret_key = VRF_HARDCODED_SECRET_KEY;
    let vrf_account_private_key = Felt::from(secret_key);
    let public_key = generate_public_key(scalar_from_felt(secret_key.into()));
    let vrf_public_key_x = felt_from_field(public_key.x)?;
    let vrf_public_key_y = felt_from_field(public_key.y)?;

    let account_public_key =
        SigningKey::from_secret_scalar(vrf_account_private_key).verifying_key().scalar();

    let vrf_account_class_hash = CartridgeVrfAccount::HASH;
    // When using UDC with unique=0 (non-unique deployment), the deployer_address
    // used in address computation is 0, not the actual deployer or UDC address.
    let vrf_account_address = get_contract_address(
        Felt::from(VRF_ACCOUNT_SALT),
        vrf_account_class_hash,
        &[account_public_key],
        ContractAddress::ZERO,
    )
    .into();

    Ok(VrfAccountCredentials {
        private_key: vrf_account_private_key,
        account_address: vrf_account_address,
        vrf_public_key_x,
        vrf_public_key_y,
        secret_key,
    })
}

pub async fn bootstrap_vrf(
    rpc_url: Url,
    bootstrapper_account_address: ContractAddress,
    bootstrapper_account_private_key: Felt,
) -> Result<VrfBootstrapResult> {
    let provider = JsonRpcClient::new(HttpTransport::new(rpc_url.clone()));

    // Get chain ID from the node
    let chain_id = provider.chain_id().await.context("failed to get chain ID from node")?;

    // Create the source account for transactions
    let signer =
        LocalWallet::from(SigningKey::from_secret_scalar(bootstrapper_account_private_key));
    let account = SingleOwnerAccount::new(
        provider.clone(),
        signer,
        bootstrapper_account_address.into(),
        chain_id,
        ExecutionEncoding::New,
    );

    let vrf_acc_cred = bootstrap_vrf_account(&account).await?;
    let vrf_consumer_addr = bootstrap_vrf_consumer(&account, vrf_acc_cred.account_address).await?;

    Ok(VrfBootstrapResult {
        chain_id: chain_id.into(),
        secret_key: vrf_acc_cred.secret_key,
        vrf_consumer_address: vrf_consumer_addr,
        vrf_account_address: vrf_acc_cred.account_address,
        vrf_account_private_key: vrf_acc_cred.private_key,
    })
}

async fn bootstrap_vrf_account(
    bootstrapper_account: &SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>,
) -> Result<VrfAccountCredentials> {
    let provider = bootstrapper_account.provider();

    let vrf_acc_cred = get_vrf_account()?;
    let vrf_account_address = vrf_acc_cred.account_address;

    if !is_declared(provider, CartridgeVrfAccount::HASH).await? {
        let class = CartridgeVrfAccount::CLASS.clone();
        let compiled_hash = CartridgeVrfAccount::CASM_HASH;

        let rpc_class = RpcSierraContractClass::from(class.to_sierra().unwrap());
        let rpc_class = FlattenedSierraClass::try_from(rpc_class).unwrap();

        let result = bootstrapper_account
            .declare_v3(rpc_class.into(), compiled_hash)
            .send()
            .await
            .expect("fail to declare class");

        assert_eq!(result.class_hash, CartridgeVrfAccount::HASH, "Class hash mismatch");
        wait_for_class(provider, CartridgeVrfAccount::HASH, BOOTSTRAP_TIMEOUT).await?;
    }

    // Deploy VRF account if not already deployed
    if !is_deployed(provider, vrf_account_address).await? {
        let account_public_key =
            SigningKey::from_secret_scalar(vrf_acc_cred.private_key).verifying_key().scalar();

        #[allow(deprecated)]
        let factory = ContractFactory::new(CartridgeVrfAccount::HASH, &bootstrapper_account);
        factory
            .deploy_v3(vec![account_public_key], Felt::from(VRF_ACCOUNT_SALT), false)
            .send()
            .await
            .map_err(|e| anyhow!("failed to deploy VRF account: {e}"))?;

        wait_for_contract(provider, vrf_account_address, BOOTSTRAP_TIMEOUT).await?;
    }

    // Fund VRF account
    {
        let amount = Felt::from(1_000_000_000_000_000_000u128);
        let transfer_call = Call {
            to: DEFAULT_STRK_FEE_TOKEN_ADDRESS.into(),
            selector: selector!("transfer"),
            calldata: vec![vrf_account_address.into(), amount, Felt::ZERO],
        };

        let result = bootstrapper_account
            .execute_v3(vec![transfer_call])
            .send()
            .await
            .map_err(|e| anyhow!("failed to fund VRF account: {e}"))?;

        wait_for_tx(provider, result.transaction_hash, BOOTSTRAP_TIMEOUT).await?;
    }

    // Set VRF public key on the deployed account
    // Create account for the VRF account to call set_vrf_public_key on itself
    let vrf_account = SingleOwnerAccount::new(
        provider,
        LocalWallet::from(SigningKey::from_secret_scalar(vrf_acc_cred.private_key)),
        vrf_account_address.into(),
        bootstrapper_account.chain_id(),
        ExecutionEncoding::New,
    );

    let set_vrf_key_call = Call {
        to: vrf_account_address.into(),
        selector: selector!("set_vrf_public_key"),
        calldata: vec![vrf_acc_cred.vrf_public_key_x, vrf_acc_cred.vrf_public_key_y],
    };

    let result = vrf_account
        .execute_v3(vec![set_vrf_key_call])
        .send()
        .await
        .map_err(|e| anyhow!("failed to set VRF public key: {e}"))?;

    wait_for_tx(provider, result.transaction_hash, BOOTSTRAP_TIMEOUT).await?;

    Ok(vrf_acc_cred)
}

async fn bootstrap_vrf_consumer(
    bootstrapper_account: &SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>,
    vrf_account: ContractAddress,
) -> Result<ContractAddress> {
    let provider = bootstrapper_account.provider();

    // When using UDC with unique=0 (non-unique deployment), the deployer_address
    // used in address computation is 0, not the actual deployer or UDC address.
    let vrf_consumer_address = get_contract_address(
        Felt::from(VRF_CONSUMER_SALT),
        CartridgeVrfConsumer::HASH,
        &[vrf_account.into()],
        ContractAddress::ZERO,
    );

    if !is_declared(provider, CartridgeVrfConsumer::HASH).await? {
        let class = CartridgeVrfConsumer::CLASS.clone();
        let compiled_hash = CartridgeVrfConsumer::CASM_HASH;

        let rpc_class = RpcSierraContractClass::from(class.to_sierra().unwrap());
        let rpc_class = FlattenedSierraClass::try_from(rpc_class).unwrap();

        let result = bootstrapper_account
            .declare_v3(rpc_class.into(), compiled_hash)
            .send()
            .await
            .expect("fail to declare class");

        assert_eq!(result.class_hash, CartridgeVrfConsumer::HASH, "Class hash mismatch");
        wait_for_class(provider, CartridgeVrfConsumer::HASH, BOOTSTRAP_TIMEOUT).await?;
    }

    if !is_deployed(provider, vrf_consumer_address.into()).await? {
        #[allow(deprecated)]
        let factory = ContractFactory::new(CartridgeVrfConsumer::HASH, &bootstrapper_account);
        factory
            .deploy_v3(vec![vrf_account.into()], Felt::from(VRF_CONSUMER_SALT), false)
            .send()
            .await
            .map_err(|e| anyhow!("failed to deploy VRF consumer: {e}"))?;

        wait_for_contract(provider, vrf_consumer_address.into(), BOOTSTRAP_TIMEOUT).await?;
    }

    Ok(vrf_consumer_address.into())
}

// ============================================================================
// Internal helpers
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

async fn wait_for_tx(
    provider: &JsonRpcClient<HttpTransport>,
    tx_hash: Felt,
    timeout: Duration,
) -> Result<()> {
    let start = Instant::now();
    loop {
        match provider.get_transaction_receipt(tx_hash).await {
            Ok(_) => return Ok(()),
            Err(ProviderError::StarknetError(StarknetError::TransactionHashNotFound)) => {}
            Err(e) => return Err(anyhow!("failed to get transaction receipt: {e}")),
        }
        if start.elapsed() > timeout {
            return Err(anyhow!("transaction {tx_hash:#x} not confirmed before timeout"));
        }
        sleep(Duration::from_millis(200)).await;
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

#[cfg(test)]
mod tests {
    use super::{get_vrf_account, VRF_HARDCODED_SECRET_KEY};

    #[test]
    fn derive_vrf_accounts_uses_hardcoded_secret_key() {
        let derived = get_vrf_account().expect("must derive");
        assert_eq!(derived.secret_key, VRF_HARDCODED_SECRET_KEY);
        assert_eq!(derived.private_key, VRF_HARDCODED_SECRET_KEY.into());
    }

    #[test]
    fn derive_vrf_accounts_is_deterministic() {
        let first = get_vrf_account().expect("first derivation");
        let second = get_vrf_account().expect("second derivation");

        assert_eq!(first.account_address, second.account_address);
        assert_eq!(first.vrf_public_key_x, second.vrf_public_key_x);
        assert_eq!(first.vrf_public_key_y, second.vrf_public_key_y);
    }
}
