//! Helpers for pre-allocating an ERC20 fee-token contract directly into genesis state.
//!
//! Bypasses on-chain UDC deployment so the contract can land at an arbitrary address (e.g. the
//! canonical Starknet mainnet STRK address), which is impossible with UDC alone because the
//! deployed address is mathematically derived from `(class_hash, salt, ctor_args, deployer)`.
//!
//! Used by both [`crate::dev::ChainSpec::state_updates`] (for ETH + STRK fee tokens) and
//! [`crate::rollup::ChainSpec::state_updates`] (for the STRK fee token).

use std::collections::BTreeMap;
use std::str::FromStr;

use alloy_primitives::U256;
use katana_genesis::allocation::GenesisAllocation;
use katana_genesis::constant::{
    get_fee_token_balance_base_storage_address, ERC20_DECIMAL_STORAGE_SLOT,
    ERC20_NAME_STORAGE_SLOT, ERC20_SYMBOL_STORAGE_SLOT, ERC20_TOTAL_SUPPLY_STORAGE_SLOT,
};
use katana_primitives::cairo::ShortString;
use katana_primitives::class::ClassHash;
use katana_primitives::contract::ContractAddress;
use katana_primitives::state::StateUpdatesWithClasses;
use katana_primitives::utils::split_u256;
use katana_primitives::Felt;

/// Writes a fee-token ERC20 contract into `states` at `address`.
///
/// Seeds balance storage for every entry in `allocations` that carries a balance, plus any extra
/// `(address, balance)` pairs in `extra_balances` — used by the rollup builder to credit its
/// transaction-genesis master account, which is not a genesis allocation.
///
/// The caller is responsible for inserting the contract class into `states.classes` and into the
/// appropriate declared-classes set; this helper only handles per-token deployment + storage.
#[allow(clippy::too_many_arguments)] // builder-style genesis helper; each arg is a distinct token field
pub(crate) fn add_fee_token(
    states: &mut StateUpdatesWithClasses,
    name: &str,
    symbol: &str,
    decimals: u8,
    address: ContractAddress,
    class_hash: ClassHash,
    allocations: &BTreeMap<ContractAddress, GenesisAllocation>,
    extra_balances: &[(ContractAddress, U256)],
) {
    let mut storage = BTreeMap::new();
    let mut total_supply = U256::ZERO;

    // --- set the ERC20 balances for each allocations that have a balance

    for (address, alloc) in allocations {
        if let Some(balance) = alloc.balance() {
            total_supply += balance;
            write_balance(&mut storage, *address, balance);
        }
    }

    for (address, balance) in extra_balances {
        total_supply += *balance;
        write_balance(&mut storage, *address, *balance);
    }

    // --- ERC20 metadata

    let name = ShortString::from_str(name).expect("valid ERC20 name");
    let symbol = ShortString::from_str(symbol).expect("valid ERC20 symbol");
    let decimals = decimals.into();
    let (total_supply_low, total_supply_high) = split_u256(total_supply);

    storage.insert(ERC20_NAME_STORAGE_SLOT, name.into());
    storage.insert(ERC20_SYMBOL_STORAGE_SLOT, symbol.into());
    storage.insert(ERC20_DECIMAL_STORAGE_SLOT, decimals);
    storage.insert(ERC20_TOTAL_SUPPLY_STORAGE_SLOT, total_supply_low);
    storage.insert(ERC20_TOTAL_SUPPLY_STORAGE_SLOT + Felt::ONE, total_supply_high);

    states.state_updates.deployed_contracts.insert(address, class_hash);
    states.state_updates.storage_updates.insert(address, storage);
}

fn write_balance(
    storage: &mut BTreeMap<katana_primitives::contract::StorageKey, Felt>,
    holder: ContractAddress,
    balance: U256,
) {
    let (low, high) = split_u256(balance);

    // the base storage address for a standard ERC20 contract balance
    let bal_base_storage_var = get_fee_token_balance_base_storage_address(holder);

    // the storage address of low u128 of the balance
    let low_bal_storage_var = bal_base_storage_var;
    // the storage address of high u128 of the balance
    let high_bal_storage_var = bal_base_storage_var + Felt::ONE;

    storage.insert(low_bal_storage_var, low);
    storage.insert(high_bal_storage_var, high);
}
