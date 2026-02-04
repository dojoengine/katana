//! VRF bootstrap and account derivation.
//!
//! This module handles:
//! - Deriving VRF accounts and keys from source accounts
//! - Deploying VRF account and consumer contracts via RPC
//! - Setting up VRF public keys on deployed accounts
//!
//! This module uses the starknet crate's account abstraction for transaction handling.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use ark_ff::PrimeField;
use katana_chain_spec::ChainSpec;
use katana_genesis::allocation::GenesisAccountAlloc;
use katana_genesis::constant::DEFAULT_STRK_FEE_TOKEN_ADDRESS;
use katana_primitives::chain::ChainId;
use katana_primitives::utils::get_contract_address;
use katana_primitives::{ContractAddress, Felt};
use stark_vrf::{generate_public_key, ScalarField};
use starknet::accounts::{Account, ExecutionEncoding, SingleOwnerAccount};
use starknet::contract::ContractFactory;
use starknet::core::types::{BlockId, BlockTag, Call, StarknetError};
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
    /// The VRF secret key (derived from source account).
    pub secret_key: u64,
    pub vrf_account_address: ContractAddress,
    pub vrf_consumer_address: ContractAddress,
    pub chain_id: ChainId,
}

/// Derived VRF account information.
#[derive(Debug, Clone)]
pub struct VrfDerivedAccounts {
    pub source_address: ContractAddress,
    pub source_private_key: Felt,
    pub vrf_account_address: ContractAddress,
    pub vrf_public_key_x: Felt,
    pub vrf_public_key_y: Felt,
    pub secret_key: u64,
}

// ============================================================================
// Bootstrap Functions
// ============================================================================

/// Derive VRF accounts from source account.
///
/// This computes the deterministic VRF account address and VRF key pair
/// from the source account's private key.
pub fn derive_vrf_accounts(
    source_address: ContractAddress,
    source_private_key: Felt,
) -> Result<VrfDerivedAccounts> {
    // vrf-server expects a u64 secret, so derive one from the account key.
    let secret_key = vrf_secret_key_from_account_key(source_private_key);
    let public_key = generate_public_key(scalar_from_felt(secret_key.into()));
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

pub async fn bootstrap_vrf(rpc_addr: SocketAddr, chain: &ChainSpec) -> Result<VrfBootstrapResult> {
    let rpc_url = Url::parse(&format!("http://{rpc_addr}"))?;
    let provider = Arc::new(JsonRpcClient::new(HttpTransport::new(rpc_url.clone())));

    let bootstrapper_account = prefunded_account(chain, 0)?;

    // Get chain ID from the node
    let chain_id_felt = provider.chain_id().await.context("failed to get chain ID from node")?;
    let chain_id = ChainId::Id(chain_id_felt);

    let derived = derive_vrf_accounts(bootstrapper_account.0, bootstrapper_account.1)?;
    let vrf_account_address = derived.vrf_account_address;
    let account_public_key =
        SigningKey::from_secret_scalar(bootstrapper_account.1).verifying_key().scalar();
    let vrf_account_class_hash = vrf_account_class_hash()?;

    // Create the source account for transactions
    let signer = LocalWallet::from(SigningKey::from_secret_scalar(bootstrapper_account.1));
    let account = SingleOwnerAccount::new(
        provider.clone(),
        signer,
        bootstrapper_account.0.into(),
        chain_id_felt,
        ExecutionEncoding::New,
    );

    // Deploy VRF account if not already deployed
    if !is_deployed(&provider, vrf_account_address).await? {
        #[allow(deprecated)]
        let factory = ContractFactory::new(vrf_account_class_hash, &account);
        factory
            .deploy_v3(vec![account_public_key], Felt::from(VRF_ACCOUNT_SALT), false)
            .send()
            .await
            .map_err(|e| anyhow!("failed to deploy VRF account: {e}"))?;

        wait_for_contract(&provider, vrf_account_address, BOOTSTRAP_TIMEOUT).await?;
    }

    // Fund VRF account
    {
        let amount = Felt::from(1_000_000_000_000_000_000u128);
        let transfer_call = Call {
            to: DEFAULT_STRK_FEE_TOKEN_ADDRESS.into(),
            selector: selector!("transfer"),
            calldata: vec![vrf_account_address.into(), amount, Felt::ZERO],
        };

        account
            .execute_v3(vec![transfer_call])
            .send()
            .await
            .map_err(|e| anyhow!("failed to fund VRF account: {e}"))?;
    }

    // Set VRF public key on the deployed account
    // Create account for the VRF account to call set_vrf_public_key on itself
    let vrf_signer = LocalWallet::from(SigningKey::from_secret_scalar(bootstrapper_account.1));
    let mut vrf_account = SingleOwnerAccount::new(
        provider.clone(),
        vrf_signer,
        vrf_account_address.into(),
        chain_id_felt,
        ExecutionEncoding::New,
    );
    vrf_account.set_block_id(BlockId::Tag(BlockTag::PreConfirmed));

    let set_vrf_key_call = Call {
        to: vrf_account_address.into(),
        selector: selector!("set_vrf_public_key"),
        calldata: vec![derived.vrf_public_key_x, derived.vrf_public_key_y],
    };

    vrf_account
        .execute_v3(vec![set_vrf_key_call])
        .send()
        .await
        .map_err(|e| anyhow!("failed to set VRF public key: {e}"))?;

    // Deploy VRF consumer
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

    if !is_deployed(&provider, vrf_consumer_address).await? {
        #[allow(deprecated)]
        let factory = ContractFactory::new(vrf_consumer_class_hash, &account);
        factory
            .deploy_v3(vec![vrf_account_address.into()], Felt::from(VRF_CONSUMER_SALT), false)
            .send()
            .await
            .map_err(|e| anyhow!("failed to deploy VRF consumer: {e}"))?;

        wait_for_contract(&provider, vrf_consumer_address, BOOTSTRAP_TIMEOUT).await?;
    }

    Ok(VrfBootstrapResult {
        secret_key: derived.secret_key,
        vrf_account_address,
        vrf_consumer_address,
        chain_id,
    })
}

/// Get the class hash of the VRF account contract.
pub fn vrf_account_class_hash() -> Result<Felt> {
    let class = katana_primitives::utils::class::parse_sierra_class(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../controller/classes/cartridge_vrf_VrfAccount.contract_class.json"
    )))?;
    class.class_hash().context("failed to compute vrf account class hash")
}

/// Get the class hash of the VRF consumer contract.
pub fn vrf_consumer_class_hash() -> Result<Felt> {
    let class = katana_primitives::utils::class::parse_sierra_class(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../controller/classes/cartridge_vrf_VrfConsumer.contract_class.json"
    )))?;
    class.class_hash().context("failed to compute vrf consumer class hash")
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

fn scalar_from_felt(value: Felt) -> ScalarField {
    let bytes = value.to_bytes_be();
    ScalarField::from_be_bytes_mod_order(&bytes)
}

/// Derive a u64 VRF secret key from an account private key.
///
/// Uses the low 64 bits of the account key.
pub fn vrf_secret_key_from_account_key(value: Felt) -> u64 {
    let bytes = value.to_bytes_be();
    let mut tail = [0_u8; 8];
    tail.copy_from_slice(&bytes[24..]);
    u64::from_be_bytes(tail)
}

fn felt_from_field<T: std::fmt::Display>(value: T) -> Result<Felt> {
    let decimal = value.to_string();
    Felt::from_dec_str(&decimal).map_err(|err| anyhow!("invalid field value: {err}"))
}

fn prefunded_account(chain_spec: &ChainSpec, index: u16) -> Result<(ContractAddress, Felt)> {
    let (address, allocation) = chain_spec
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

#[cfg(test)]
mod tests {
    use katana_primitives::Felt;

    use super::vrf_secret_key_from_account_key;

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
