use katana_genesis::Genesis;
use katana_primitives::chain::ChainId;
use katana_primitives::ContractAddress;
use serde::{Deserialize, Serialize};

pub mod dev;
mod fee_token;
pub mod full_node;
pub mod rollup;
pub mod settlement;
pub mod settlement_check;
pub mod tee;

pub use settlement::{SettlementConfig, SettlementLayer, SettlementProofKind, SettlementRuntime};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainSpec {
    Dev(dev::ChainSpec),
    Rollup(rollup::ChainSpec),
    FullNode(full_node::ChainSpec),
}

//////////////////////////////////////////////////////////////
// 	ChainSpec implementations
//////////////////////////////////////////////////////////////

impl ChainSpec {
    /// Creates a new [`ChainSpec`] for a development chain.
    pub fn dev() -> Self {
        Self::Dev(dev::DEV.clone())
    }

    /// Creates a new [`ChainSpec`] for Starknet mainnet.
    pub fn mainnet() -> Self {
        Self::FullNode(full_node::ChainSpec::mainnet())
    }

    /// Creates a new [`ChainSpec`] for Starknet sepolia testnet.
    pub fn sepolia() -> Self {
        Self::FullNode(full_node::ChainSpec::sepolia())
    }

    pub fn id(&self) -> ChainId {
        match self {
            Self::Dev(spec) => spec.id,
            Self::Rollup(spec) => spec.id,
            Self::FullNode(spec) => spec.id,
        }
    }

    pub fn genesis(&self) -> &Genesis {
        match self {
            Self::Dev(spec) => &spec.genesis,
            Self::Rollup(spec) => &spec.genesis,
            Self::FullNode(spec) => &spec.genesis,
        }
    }

    pub fn fee_contracts(&self) -> &FeeContracts {
        match self {
            Self::Dev(spec) => &spec.fee_contracts,
            Self::Rollup(spec) => &spec.fee_contracts,
            Self::FullNode(spec) => &spec.fee_contracts,
        }
    }
}

impl From<dev::ChainSpec> for ChainSpec {
    fn from(spec: dev::ChainSpec) -> Self {
        Self::Dev(spec)
    }
}

impl From<rollup::ChainSpec> for ChainSpec {
    fn from(spec: rollup::ChainSpec) -> Self {
        Self::Rollup(spec)
    }
}

impl From<full_node::ChainSpec> for ChainSpec {
    fn from(spec: full_node::ChainSpec) -> Self {
        Self::FullNode(spec)
    }
}

impl Default for ChainSpec {
    fn default() -> Self {
        Self::dev()
    }
}

/// Tokens that can be used for transaction fee payments in the chain. As
/// supported on Starknet.
// TODO: include both l1 and l2 addresses
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct FeeContracts {
    /// L2 ETH fee token address. Used for paying pre-V3 transactions.
    pub eth: ContractAddress,
    /// L2 STRK fee token address. Used for paying V3 transactions.
    pub strk: ContractAddress,
}
