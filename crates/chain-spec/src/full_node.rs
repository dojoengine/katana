use katana_genesis::Genesis;
use katana_primitives::block::{ExecutableBlock, GasPrices, PartialHeader};
use katana_primitives::chain::ChainId;
use katana_primitives::da::L1DataAvailabilityMode;
use katana_primitives::state::StateUpdatesWithClasses;
use katana_primitives::version::CURRENT_STARKNET_VERSION;
use lazy_static::lazy_static;

use crate::{ChainSpecT, FeeContracts, SettlementLayer};

/// The full node chain specification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainSpec {
    /// The network chain id.
    pub id: ChainId,

    /// The chain's genesis states.
    pub genesis: Genesis,

    /// The chain fee token contract.
    pub fee_contracts: FeeContracts,

    /// The chain's settlement layer configurations (if any).
    pub settlement: Option<SettlementLayer>,
}

//////////////////////////////////////////////////////////////
// 	ChainSpec implementations
//////////////////////////////////////////////////////////////

impl ChainSpec {
    /// Creates a new [`ChainSpec`] for Starknet mainnet.
    pub fn mainnet() -> Self {
        MAINNET.clone()
    }

    /// Creates a new [`ChainSpec`] for Starknet sepolia testnet.
    pub fn sepolia() -> Self {
        SEPOLIA.clone()
    }

    pub fn block(&self) -> ExecutableBlock {
        let header = PartialHeader {
            starknet_version: CURRENT_STARKNET_VERSION,
            number: self.genesis.number,
            timestamp: self.genesis.timestamp,
            parent_hash: self.genesis.parent_hash,
            l1_da_mode: L1DataAvailabilityMode::Calldata,
            l2_gas_prices: GasPrices::MIN,
            l1_gas_prices: self.genesis.gas_prices.clone(),
            l1_data_gas_prices: self.genesis.gas_prices.clone(),
            sequencer_address: self.genesis.sequencer_address,
        };

        ExecutableBlock { header, body: Vec::new() }
    }

    pub fn state_updates(&self) -> StateUpdatesWithClasses {
        // Full node chain spec syncs from the network, so we return empty state updates here.
        StateUpdatesWithClasses::default()
    }
}

impl ChainSpecT for ChainSpec {
    fn id(&self) -> ChainId {
        self.id
    }

    fn genesis(&self) -> &Genesis {
        &self.genesis
    }

    fn fee_contracts(&self) -> &FeeContracts {
        &self.fee_contracts
    }

    fn settlement(&self) -> Option<&SettlementLayer> {
        self.settlement.as_ref()
    }

    fn block(&self) -> ExecutableBlock {
        self.block()
    }

    fn state_updates(&self) -> StateUpdatesWithClasses {
        self.state_updates()
    }
}

//////////////////////////////////////////////////////////////
// 	Predefined ChainSpec instances
//////////////////////////////////////////////////////////////

lazy_static! {
    /// Starknet mainnet chain specification.
    pub static ref MAINNET: ChainSpec = ChainSpec {
        id: ChainId::MAINNET,
        genesis: Genesis::default(),
        fee_contracts: FeeContracts {
            eth: katana_genesis::constant::DEFAULT_ETH_FEE_TOKEN_ADDRESS,
            strk: katana_genesis::constant::DEFAULT_STRK_FEE_TOKEN_ADDRESS,
        },
        settlement: None,
    };

    /// Starknet sepolia testnet chain specification.
    pub static ref SEPOLIA: ChainSpec = ChainSpec {
        id: ChainId::SEPOLIA,
        genesis: Genesis::default(),
        fee_contracts: FeeContracts {
            eth: katana_genesis::constant::DEFAULT_ETH_FEE_TOKEN_ADDRESS,
            strk: katana_genesis::constant::DEFAULT_STRK_FEE_TOKEN_ADDRESS,
        },
        settlement: None,
    };
}
