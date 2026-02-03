//! VRF account derivation utilities.
//!
//! This module provides utilities for deriving VRF-related accounts.
//! Used by CartridgeApi for VRF configuration.

use anyhow::{anyhow, Result};
use ark_ff::PrimeField;
use katana_core::backend::Backend;
use katana_executor::ExecutorFactory;
use katana_genesis::allocation::GenesisAccountAlloc;
use katana_genesis::constant::DEFAULT_UDC_ADDRESS;
use katana_primitives::utils::get_contract_address;
use katana_primitives::{ContractAddress, Felt};
use katana_provider::ProviderFactory;
use stark_vrf::{generate_public_key, ScalarField};
use starknet::signers::SigningKey;

use crate::config::paymaster::VrfConfig;
use crate::config::Config;

const VRF_ACCOUNT_SALT: u64 = 0x54321;

/// Derived VRF account information.
#[derive(Debug)]
pub struct VrfDerivedAccounts {
    pub source_address: ContractAddress,
    pub source_private_key: Felt,
    pub vrf_account_address: ContractAddress,
    pub vrf_public_key_x: Felt,
    pub vrf_public_key_y: Felt,
    pub secret_key: u64,
}

/// Derive VRF accounts from the configuration.
///
/// This is used by CartridgeApi to configure VRF settings.
pub fn derive_vrf_accounts<EF, PF>(
    _config: &VrfConfig,
    _node_config: &Config,
    backend: &Backend<EF, PF>,
) -> Result<VrfDerivedAccounts>
where
    EF: ExecutorFactory,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: katana_provider::ProviderRO,
    <PF as ProviderFactory>::ProviderMut: katana_provider::ProviderRW,
{
    // Always use the first genesis account (index 0)
    let (source_address, source_private_key) = prefunded_account(backend, 0)?;

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

fn vrf_account_class_hash() -> Result<Felt> {
    use anyhow::Context;
    let class = katana_primitives::utils::class::parse_sierra_class(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../controller/classes/cartridge_vrf_VrfAccount.contract_class.json"
    )))?;
    class.class_hash().context("failed to compute vrf account class hash")
}

fn scalar_from_felt(value: Felt) -> ScalarField {
    let bytes = value.to_bytes_be();
    ScalarField::from_be_bytes_mod_order(&bytes)
}

fn vrf_secret_key_from_account_key(value: Felt) -> u64 {
    let bytes = value.to_bytes_be();
    let mut tail = [0_u8; 8];
    tail.copy_from_slice(&bytes[24..]);
    u64::from_be_bytes(tail)
}

fn felt_from_field<T: std::fmt::Display>(value: T) -> Result<Felt> {
    let decimal = value.to_string();
    Felt::from_dec_str(&decimal).map_err(|err| anyhow!("invalid field value: {err}"))
}
