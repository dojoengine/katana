pub mod allocation;
pub mod constant;
pub mod json;

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::Arc;

use katana_contracts::contracts::{Account, LegacyERC20, UniversalDeployer};
use katana_primitives::block::{BlockHash, BlockNumber, GasPrices};
use katana_primitives::class::{ClassHash, ContractClass};
use katana_primitives::contract::ContractAddress;
use katana_primitives::Felt;
use serde::{Deserialize, Serialize};

use self::allocation::{GenesisAccountAlloc, GenesisAllocation, GenesisContractAlloc};
use crate::allocation::DevGenesisAccount;

/// Genesis block configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Genesis {
    /// The genesis block parent hash.
    pub parent_hash: BlockHash,
    /// The genesis block state root.
    pub state_root: Felt,
    /// The genesis block number.
    pub number: BlockNumber,
    /// The genesis block timestamp.
    pub timestamp: u64,
    /// The genesis block sequencer address.
    pub sequencer_address: ContractAddress,
    /// The genesis block L1 gas prices.
    pub gas_prices: GasPrices,
    /// The classes to declare in the genesis block.
    pub classes: BTreeMap<ClassHash, Arc<ContractClass>>,
    /// The genesis contract allocations.
    pub allocations: BTreeMap<ContractAddress, GenesisAllocation>,
}

impl Genesis {
    /// Extends the genesis allocations with the given allocations.
    pub fn extend_allocations<T>(&mut self, allocs: T)
    where
        T: IntoIterator<Item = (ContractAddress, GenesisAllocation)>,
    {
        self.allocations.extend(allocs);
    }

    /// Returns an iterator over the generic (non-account) contracts.
    pub fn contracts(&self) -> impl Iterator<Item = &GenesisContractAlloc> {
        self.allocations.values().filter_map(|allocation| {
            if let GenesisAllocation::Contract(contract) = allocation {
                Some(contract)
            } else {
                None
            }
        })
    }

    /// Returns an iterator over the genesis accounts. This will only return
    /// allocated account contracts.
    pub fn accounts(&self) -> impl Iterator<Item = (&ContractAddress, &GenesisAccountAlloc)> {
        self.allocations.iter().filter_map(|(addr, alloc)| {
            if let GenesisAllocation::Account(account) = alloc {
                Some((addr, account))
            } else {
                None
            }
        })
    }

    pub fn paymaster_account(&self) -> Option<(ContractAddress, &DevGenesisAccount)> {
        // TODO: deploy a dedicated account for the paymaster instead of using the first dev account
        match self.accounts().nth(1) {
            Some((addr, account)) => {
                if let GenesisAccountAlloc::DevAccount(account) = account {
                    Some((*addr, account))
                } else {
                    None
                }
            }
            None => None,
        }
    }
}

impl Default for Genesis {
    /// Creates a new [Genesis] with the default configurations and classes. The default
    /// classes are a legacy ERC20 class for the fee token, a legacy UDC class for the
    /// universal deployer, and an OpenZeppelin account contract class.
    fn default() -> Self {
        let mut classes = BTreeMap::new();

        classes.extend(BTreeMap::from([
            // Fee token class
            (LegacyERC20::HASH, LegacyERC20::CLASS.clone().into()),
            // universal depoyer contract class
            (UniversalDeployer::HASH, UniversalDeployer::CLASS.clone().into()),
            // predeployed account class
            (Account::HASH, Account::CLASS.clone().into()),
        ]));

        Self {
            parent_hash: Felt::ZERO,
            number: 0,
            state_root: Felt::ZERO,
            timestamp: 0,
            gas_prices: GasPrices::default(),
            sequencer_address: Felt::ZERO.into(),
            classes,
            allocations: BTreeMap::new(),
        }
    }
}
