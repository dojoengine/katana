use std::collections::BTreeMap;

use alloy_primitives::U256;
use katana_contracts::contracts;
use katana_primitives::block::{ExecutableBlock, GasPrices, PartialHeader};
use katana_primitives::chain::ChainId;
use katana_primitives::class::ClassHash;
use katana_primitives::contract::ContractAddress;
use katana_primitives::da::L1DataAvailabilityMode;
use katana_primitives::genesis::allocation::{DevAllocationsGenerator, GenesisAllocation};
use katana_primitives::genesis::constant::{
    get_fee_token_balance_base_storage_address, DEFAULT_ACCOUNT_CLASS_PUBKEY_STORAGE_SLOT,
    DEFAULT_ETH_FEE_TOKEN_ADDRESS, DEFAULT_PREFUNDED_ACCOUNT_BALANCE,
    DEFAULT_STRK_FEE_TOKEN_ADDRESS, DEFAULT_UDC_ADDRESS, ERC20_DECIMAL_STORAGE_SLOT,
    ERC20_NAME_STORAGE_SLOT, ERC20_SYMBOL_STORAGE_SLOT, ERC20_TOTAL_SUPPLY_STORAGE_SLOT,
};
use katana_primitives::genesis::Genesis;
use katana_primitives::state::StateUpdatesWithClasses;
use katana_primitives::utils::split_u256;
use katana_primitives::version::StarknetVersion;
use katana_primitives::Felt;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use starknet::core::utils::cairo_short_string_to_felt;

use crate::SettlementLayer;

#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq))]
pub struct ChainSpec {
    /// The rollup network chain id.
    pub id: ChainId,

    /// The chain's genesis states.
    pub genesis: Genesis,

    /// The chain fee token contract.
    pub fee_contracts: FeeContracts,

    pub settlement: Option<SettlementLayer>,
}

//////////////////////////////////////////////////////////////
// 	ChainSpec implementations
//////////////////////////////////////////////////////////////

impl ChainSpec {
    pub fn block(&self) -> ExecutableBlock {
        let header = PartialHeader {
            number: self.genesis.number,
            timestamp: self.genesis.timestamp,
            parent_hash: self.genesis.parent_hash,
            l1_da_mode: L1DataAvailabilityMode::Calldata,
            l1_gas_prices: self.genesis.gas_prices.clone(),
            l2_gas_prices: GasPrices::MIN,
            l1_data_gas_prices: self.genesis.gas_prices.clone(),
            sequencer_address: self.genesis.sequencer_address,
            // keep at 0.13.1.1 for backward compatibility reason
            starknet_version: StarknetVersion::new([0, 13, 1, 1]),
        };

        ExecutableBlock { header, body: Vec::new() }
    }

    // this method will include the ETH and STRK fee tokens, and the UDC
    pub fn state_updates(&self) -> StateUpdatesWithClasses {
        let mut states = StateUpdatesWithClasses::default();

        for (class_hash, class) in &self.genesis.classes {
            let class_hash = *class_hash;

            if class.is_legacy() {
                states.state_updates.deprecated_declared_classes.insert(class_hash);
            } else {
                let casm_hash = class.as_ref().clone().compile().unwrap().class_hash().unwrap();
                states.state_updates.declared_classes.insert(class_hash, casm_hash);
            }

            states.classes.insert(class_hash, class.as_ref().clone());
        }

        for (address, alloc) in &self.genesis.allocations {
            let address = *address;

            if let Some(hash) = alloc.class_hash() {
                states.state_updates.deployed_contracts.insert(address, hash);
            }

            if let Some(nonce) = alloc.nonce() {
                states.state_updates.nonce_updates.insert(address, nonce);
            }

            let mut storage = alloc.storage().cloned().unwrap_or_default();
            if let Some(pub_key) = alloc.public_key() {
                storage.insert(DEFAULT_ACCOUNT_CLASS_PUBKEY_STORAGE_SLOT, pub_key);
            }

            states.state_updates.storage_updates.insert(address, storage);
        }

        //-- Fee tokens
        add_default_fee_tokens(&mut states, &self.genesis);
        // -- UDC
        add_default_udc(&mut states);

        states
    }
}

/// Tokens that can be used for transaction fee payments in the chain. As
/// supported on Starknet.
// TODO: include both l1 and l2 addresses
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(PartialEq))]
pub struct FeeContracts {
    /// L2 ETH fee token address. Used for paying pre-V3 transactions.
    pub eth: ContractAddress,
    /// L2 STRK fee token address. Used for paying V3 transactions.
    pub strk: ContractAddress,
}

impl Default for ChainSpec {
    fn default() -> Self {
        DEV.clone()
    }
}

lazy_static! {
    /// The default chain specification in dev mode.
    pub static ref DEV: ChainSpec = {
        let mut chain_spec = DEV_UNALLOCATED.clone();

        let accounts = DevAllocationsGenerator::new(10)
            .with_balance(U256::from(DEFAULT_PREFUNDED_ACCOUNT_BALANCE))
            .generate();

        chain_spec.genesis.extend_allocations(accounts.into_iter().map(|(k, v)| (k, v.into())));
        chain_spec
    };

    /// The default chain specification for dev mode but without any allocations.
    ///
    /// Used when we want to create a chain spec with user defined # of allocations.
    pub static ref DEV_UNALLOCATED: ChainSpec = {
        let id = ChainId::parse("KATANA").unwrap();
        let genesis = Genesis::default();
        let fee_contracts = FeeContracts { eth: DEFAULT_ETH_FEE_TOKEN_ADDRESS, strk: DEFAULT_STRK_FEE_TOKEN_ADDRESS };

        ChainSpec {
            id,
            genesis,
            fee_contracts,
            settlement: None,
        }
    };
}

fn add_default_fee_tokens(states: &mut StateUpdatesWithClasses, genesis: &Genesis) {
    // declare erc20 token contract
    states
        .classes
        .entry(contracts::LegacyERC20::HASH)
        .or_insert_with(|| contracts::LegacyERC20::CLASS.clone());
    states.state_updates.deprecated_declared_classes.insert(contracts::LegacyERC20::HASH);

    // -- ETH
    add_fee_token(
        states,
        "Ether",
        "ETH",
        18,
        DEFAULT_ETH_FEE_TOKEN_ADDRESS,
        contracts::LegacyERC20::HASH,
        &genesis.allocations,
    );

    // -- STRK
    add_fee_token(
        states,
        "Starknet Token",
        "STRK",
        18,
        DEFAULT_STRK_FEE_TOKEN_ADDRESS,
        contracts::LegacyERC20::HASH,
        &genesis.allocations,
    );
}

fn add_fee_token(
    states: &mut StateUpdatesWithClasses,
    name: &str,
    symbol: &str,
    decimals: u8,
    address: ContractAddress,
    class_hash: ClassHash,
    allocations: &BTreeMap<ContractAddress, GenesisAllocation>,
) {
    let mut storage = BTreeMap::new();
    let mut total_supply = U256::ZERO;

    // --- set the ERC20 balances for each allocations that have a balance

    for (address, alloc) in allocations {
        if let Some(balance) = alloc.balance() {
            total_supply += balance;
            let (low, high) = split_u256(balance);

            // the base storage address for a standard ERC20 contract balance
            let bal_base_storage_var = get_fee_token_balance_base_storage_address(*address);

            // the storage address of low u128 of the balance
            let low_bal_storage_var = bal_base_storage_var;
            // the storage address of high u128 of the balance
            let high_bal_storage_var = bal_base_storage_var + Felt::ONE;

            storage.insert(low_bal_storage_var, low);
            storage.insert(high_bal_storage_var, high);
        }
    }

    // --- ERC20 metadata

    let name = cairo_short_string_to_felt(name).unwrap();
    let symbol = cairo_short_string_to_felt(symbol).unwrap();
    let decimals = decimals.into();
    let (total_supply_low, total_supply_high) = split_u256(total_supply);

    storage.insert(ERC20_NAME_STORAGE_SLOT, name);
    storage.insert(ERC20_SYMBOL_STORAGE_SLOT, symbol);
    storage.insert(ERC20_DECIMAL_STORAGE_SLOT, decimals);
    storage.insert(ERC20_TOTAL_SUPPLY_STORAGE_SLOT, total_supply_low);
    storage.insert(ERC20_TOTAL_SUPPLY_STORAGE_SLOT + Felt::ONE, total_supply_high);

    states.state_updates.deployed_contracts.insert(address, class_hash);
    states.state_updates.storage_updates.insert(address, storage);
}

fn add_default_udc(states: &mut StateUpdatesWithClasses) {
    // declare UDC class
    states
        .classes
        .entry(contracts::UniversalDeployer::HASH)
        .or_insert_with(|| contracts::UniversalDeployer::CLASS.clone());

    states.state_updates.deprecated_declared_classes.insert(contracts::UniversalDeployer::HASH);

    // deploy UDC contract
    states
        .state_updates
        .deployed_contracts
        .entry(DEFAULT_UDC_ADDRESS)
        .or_insert(contracts::UniversalDeployer::HASH);
}

#[cfg(test)]
mod tests {

    use std::str::FromStr;

    use alloy_primitives::U256;
    use katana_primitives::address;
    use katana_primitives::block::GasPrices;
    use katana_primitives::da::L1DataAvailabilityMode;
    use katana_primitives::genesis::allocation::{
        GenesisAccount, GenesisAccountAlloc, GenesisContractAlloc,
    };
    use katana_primitives::genesis::constant::DEFAULT_ACCOUNT_CLASS_PUBKEY_STORAGE_SLOT;
    use starknet::macros::felt;

    use super::*;

    #[test]
    fn genesis_block_and_state_updates() {
        // setup initial states to test

        let classes = BTreeMap::from([
            (contracts::LegacyERC20::HASH, contracts::LegacyERC20::CLASS.clone().into()),
            (
                contracts::UniversalDeployer::HASH,
                contracts::UniversalDeployer::CLASS.clone().into(),
            ),
            (contracts::Account::HASH, contracts::Account::CLASS.clone().into()),
        ]);

        let allocations = [
            (
                address!("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
                GenesisAllocation::Account(GenesisAccountAlloc::Account(GenesisAccount {
                    public_key: felt!(
                        "0x01ef15c18599971b7beced415a40f0c7deacfd9b0d1819e03d723d8bc943cfca"
                    ),
                    balance: Some(U256::from_str("0xD3C21BCECCEDA1000000").unwrap()),
                    class_hash: contracts::Account::HASH,
                    nonce: Some(felt!("0x99")),
                    storage: Some(BTreeMap::from([
                        (felt!("0x1"), felt!("0x1")),
                        (felt!("0x2"), felt!("0x2")),
                    ])),
                    salt: GenesisAccount::DEFAULT_SALT,
                })),
            ),
            (
                address!("0xdeadbeef"),
                GenesisAllocation::Contract(GenesisContractAlloc {
                    balance: Some(U256::from_str("0xD3C21BCECCEDA1000000").unwrap()),
                    class_hash: Some(contracts::Account::HASH),
                    nonce: Some(felt!("0x100")),
                    storage: Some(BTreeMap::from([
                        (felt!("0x100"), felt!("0x111")),
                        (felt!("0x200"), felt!("0x222")),
                    ])),
                }),
            ),
            (
                address!("0x2"),
                GenesisAllocation::Account(GenesisAccountAlloc::Account(GenesisAccount {
                    public_key: felt!("0x2"),
                    balance: Some(U256::ZERO),
                    class_hash: contracts::Account::HASH,
                    nonce: None,
                    storage: None,
                    salt: GenesisAccount::DEFAULT_SALT,
                })),
            ),
        ];
        let chain_spec = ChainSpec {
            id: ChainId::SEPOLIA,
            genesis: Genesis {
                classes,
                allocations: BTreeMap::from(allocations.clone()),
                number: 0,
                timestamp: 5123512314u64,
                state_root: felt!("0x99"),
                parent_hash: felt!("0x999"),
                sequencer_address: address!("0x100"),
                gas_prices: unsafe { GasPrices::new_unchecked(1111, 2222) },
            },
            fee_contracts: FeeContracts {
                eth: DEFAULT_ETH_FEE_TOKEN_ADDRESS,
                strk: DEFAULT_STRK_FEE_TOKEN_ADDRESS,
            },
            settlement: None,
        };

        // setup expected storage values
        let expected_block = ExecutableBlock {
            header: PartialHeader {
                number: chain_spec.genesis.number,
                timestamp: chain_spec.genesis.timestamp,
                parent_hash: chain_spec.genesis.parent_hash,
                sequencer_address: chain_spec.genesis.sequencer_address,
                l2_gas_prices: GasPrices::MIN,
                l1_gas_prices: chain_spec.genesis.gas_prices.clone(),
                l1_data_gas_prices: chain_spec.genesis.gas_prices.clone(),
                l1_da_mode: L1DataAvailabilityMode::Calldata,
                starknet_version: StarknetVersion::new([0, 13, 1, 1]),
            },
            body: Vec::new(),
        };

        let actual_block = chain_spec.block();
        let actual_state_updates = chain_spec.state_updates();

        similar_asserts::assert_eq!(actual_block, expected_block);

        assert!(actual_state_updates.classes.len() == 3);

        assert_eq!(
            actual_state_updates.state_updates.declared_classes.get(&contracts::LegacyERC20::HASH),
            None,
            "The default erc20 is a legacy class - legacy class should only be in \
             `deprecated_declared_classes`"
        );
        assert_eq!(
            actual_state_updates
                .state_updates
                .deprecated_declared_classes
                .get(&contracts::LegacyERC20::HASH),
            Some(&contracts::LegacyERC20::CASM_HASH),
        );
        assert_eq!(
            actual_state_updates.classes.get(&contracts::LegacyERC20::HASH),
            Some(&contracts::LegacyERC20::CLASS.clone())
        );

        assert_eq!(
            actual_state_updates
                .state_updates
                .deployed_contracts
                .get(&DEFAULT_ETH_FEE_TOKEN_ADDRESS),
            Some(&contracts::LegacyERC20::HASH),
            "The ETH fee token contract should be created"
        );
        assert_eq!(
            actual_state_updates
                .state_updates
                .deployed_contracts
                .get(&DEFAULT_STRK_FEE_TOKEN_ADDRESS),
            Some(&contracts::LegacyERC20::HASH),
            "The STRK fee token contract should be created"
        );

        assert_eq!(
            actual_state_updates
                .state_updates
                .deprecated_declared_classes
                .get(&contracts::UniversalDeployer::HASH),
            Some(&contracts::UniversalDeployer::HASH),
            "The default universal deployer class should be declared"
        );

        assert_eq!(
            actual_state_updates
                .state_updates
                .declared_classes
                .get(&contracts::UniversalDeployer::HASH),
            None,
            "The udc is a legacy class - legacy class should only be in \
             `deprecated_declared_classes`"
        );
        assert_eq!(
            actual_state_updates.classes.get(&contracts::UniversalDeployer::HASH),
            Some(&contracts::UniversalDeployer::CLASS.clone())
        );

        assert_eq!(
            actual_state_updates.state_updates.deployed_contracts.get(&DEFAULT_UDC_ADDRESS),
            Some(&contracts::UniversalDeployer::HASH),
            "The universal deployer contract should be created"
        );

        assert_eq!(
            actual_state_updates.state_updates.declared_classes.get(&contracts::Account::HASH),
            Some(&contracts::Account::CASM_HASH),
            "The default oz account class should be declared"
        );

        assert_eq!(
            actual_state_updates.classes.get(&contracts::Account::HASH),
            Some(&contracts::Account::CLASS.clone()),
            "The default oz account contract sierra class should be declared"
        );

        // check that all contract allocations exist in the state updates

        assert_eq!(
            actual_state_updates.state_updates.deployed_contracts.len(),
            6,
            "6 contracts should be created: STRK fee token, ETH fee token, universal deployer, \
             and 3 allocations"
        );

        let alloc_1_addr = allocations[0].0;

        let mut account_allocation_storage = allocations[0].1.storage().unwrap().clone();
        account_allocation_storage.insert(
            DEFAULT_ACCOUNT_CLASS_PUBKEY_STORAGE_SLOT,
            felt!("0x01ef15c18599971b7beced415a40f0c7deacfd9b0d1819e03d723d8bc943cfca"),
        );

        assert_eq!(
            actual_state_updates.state_updates.deployed_contracts.get(&alloc_1_addr),
            allocations[0].1.class_hash().as_ref(),
            "allocation should exist"
        );
        assert_eq!(
            actual_state_updates.state_updates.nonce_updates.get(&alloc_1_addr).cloned(),
            allocations[0].1.nonce(),
            "allocation nonce should be updated"
        );
        assert_eq!(
            actual_state_updates.state_updates.storage_updates.get(&alloc_1_addr).cloned(),
            Some(account_allocation_storage),
            "account allocation storage should be updated"
        );

        let alloc_2_addr = allocations[1].0;

        assert_eq!(
            actual_state_updates.state_updates.deployed_contracts.get(&alloc_2_addr),
            allocations[1].1.class_hash().as_ref(),
            "allocation should exist"
        );
        assert_eq!(
            actual_state_updates.state_updates.nonce_updates.get(&alloc_2_addr).cloned(),
            allocations[1].1.nonce(),
            "allocation nonce should be updated"
        );
        assert_eq!(
            actual_state_updates.state_updates.storage_updates.get(&alloc_2_addr),
            allocations[1].1.storage(),
            "allocation storage should be updated"
        );

        let alloc_3_addr = allocations[2].0;

        assert_eq!(
            actual_state_updates.state_updates.deployed_contracts.get(&alloc_3_addr),
            allocations[2].1.class_hash().as_ref(),
            "allocation should exist"
        );
        assert_eq!(
            actual_state_updates.state_updates.nonce_updates.get(&alloc_3_addr).cloned(),
            allocations[2].1.nonce(),
            "allocation nonce should be updated"
        );
        assert_eq!(
            actual_state_updates.state_updates.storage_updates.get(&alloc_3_addr).cloned(),
            Some(BTreeMap::from([(DEFAULT_ACCOUNT_CLASS_PUBKEY_STORAGE_SLOT, felt!("0x2"))])),
            "account allocation storage should be updated"
        );

        // check ETH fee token contract storage

        // there are only two allocations with a balance so the total token supply is
        // 0xD3C21BCECCEDA1000000 * 2 = 0x1a784379d99db42000000
        let (total_supply_low, total_supply_high) =
            split_u256(U256::from_str("0x1a784379d99db42000000").unwrap());

        let name = cairo_short_string_to_felt("Ether").unwrap();
        let symbol = cairo_short_string_to_felt("ETH").unwrap();
        let decimals = Felt::from(18);

        let eth_fee_token_storage = actual_state_updates
            .state_updates
            .storage_updates
            .get(&DEFAULT_ETH_FEE_TOKEN_ADDRESS)
            .unwrap();

        assert_eq!(eth_fee_token_storage.get(&ERC20_NAME_STORAGE_SLOT), Some(&name));
        assert_eq!(eth_fee_token_storage.get(&ERC20_SYMBOL_STORAGE_SLOT), Some(&symbol));
        assert_eq!(eth_fee_token_storage.get(&ERC20_DECIMAL_STORAGE_SLOT), Some(&decimals));
        assert_eq!(
            eth_fee_token_storage.get(&ERC20_TOTAL_SUPPLY_STORAGE_SLOT),
            Some(&total_supply_low)
        );
        assert_eq!(
            eth_fee_token_storage.get(&(ERC20_TOTAL_SUPPLY_STORAGE_SLOT + Felt::ONE)),
            Some(&total_supply_high)
        );

        // check STRK fee token contract storage

        let strk_name = cairo_short_string_to_felt("Starknet Token").unwrap();
        let strk_symbol = cairo_short_string_to_felt("STRK").unwrap();
        let strk_decimals = Felt::from(18);

        let strk_fee_token_storage = actual_state_updates
            .state_updates
            .storage_updates
            .get(&DEFAULT_STRK_FEE_TOKEN_ADDRESS)
            .unwrap();

        assert_eq!(strk_fee_token_storage.get(&ERC20_NAME_STORAGE_SLOT), Some(&strk_name));
        assert_eq!(strk_fee_token_storage.get(&ERC20_SYMBOL_STORAGE_SLOT), Some(&strk_symbol));
        assert_eq!(strk_fee_token_storage.get(&ERC20_DECIMAL_STORAGE_SLOT), Some(&strk_decimals));
        assert_eq!(
            strk_fee_token_storage.get(&ERC20_TOTAL_SUPPLY_STORAGE_SLOT),
            Some(&total_supply_low)
        );
        assert_eq!(
            strk_fee_token_storage.get(&(ERC20_TOTAL_SUPPLY_STORAGE_SLOT + Felt::ONE)),
            Some(&total_supply_high)
        );

        let mut allocs_total_supply = U256::ZERO;

        // check for balance in both ETH and STRK
        for (address, alloc) in &allocations {
            if let Some(balance) = alloc.balance() {
                let (low, high) = split_u256(balance);

                // the base storage address for a standard ERC20 contract balance
                let bal_base_storage_var = get_fee_token_balance_base_storage_address(*address);

                // the storage address of low u128 of the balance
                let low_bal_storage_var = bal_base_storage_var;
                // the storage address of high u128 of the balance
                let high_bal_storage_var = bal_base_storage_var + Felt::ONE;

                assert_eq!(eth_fee_token_storage.get(&low_bal_storage_var), Some(&low));
                assert_eq!(eth_fee_token_storage.get(&high_bal_storage_var), Some(&high));

                assert_eq!(strk_fee_token_storage.get(&low_bal_storage_var), Some(&low));
                assert_eq!(strk_fee_token_storage.get(&high_bal_storage_var), Some(&high));

                allocs_total_supply += balance;
            }
        }
        // Check that the total supply is the sum of all balances in the allocations.
        // Technically this is not necessary bcs we already checked the total supply in
        // the fee token storage but it's a good sanity check.

        let (actual_total_supply_low, actual_total_supply_high) = split_u256(allocs_total_supply);
        assert_eq!(
            eth_fee_token_storage.get(&ERC20_TOTAL_SUPPLY_STORAGE_SLOT),
            Some(&actual_total_supply_low),
            "ETH total supply must be calculated from allocations balances correctly"
        );
        assert_eq!(
            eth_fee_token_storage.get(&(ERC20_TOTAL_SUPPLY_STORAGE_SLOT + Felt::ONE)),
            Some(&actual_total_supply_high),
            "ETH total supply must be calculated from allocations balances correctly"
        );

        assert_eq!(
            strk_fee_token_storage.get(&ERC20_TOTAL_SUPPLY_STORAGE_SLOT),
            Some(&actual_total_supply_low),
            "STRK total supply must be calculated from allocations balances correctly"
        );
        assert_eq!(
            strk_fee_token_storage.get(&(ERC20_TOTAL_SUPPLY_STORAGE_SLOT + Felt::ONE)),
            Some(&actual_total_supply_high),
            "STRK total supply must be calculated from allocations balances correctly"
        );
    }
}
