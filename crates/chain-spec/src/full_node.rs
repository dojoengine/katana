use katana_genesis::Genesis;
use katana_primitives::chain::ChainId;
use lazy_static::lazy_static;

use crate::{FeeContracts, SettlementLayer};

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
