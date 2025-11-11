use katana_genesis::Genesis;
use katana_primitives::block::BlockNumber;
use katana_primitives::chain::ChainId;
use katana_primitives::{eth, ContractAddress};
use serde::{Deserialize, Serialize};
use url::Url;

pub mod dev;
pub mod rollup;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainSpec {
    Dev(dev::ChainSpec),
    Rollup(rollup::ChainSpec),
}

//////////////////////////////////////////////////////////////
// 	ChainSpec implementations
//////////////////////////////////////////////////////////////

impl ChainSpec {
    /// Creates a new [`ChainSpec`] for a development chain.
    pub fn dev() -> Self {
        Self::Dev(dev::DEV.clone())
    }

    pub fn id(&self) -> ChainId {
        match self {
            Self::Dev(spec) => spec.id,
            Self::Rollup(spec) => spec.id,
        }
    }

    pub fn genesis(&self) -> &Genesis {
        match self {
            Self::Dev(spec) => &spec.genesis,
            Self::Rollup(spec) => &spec.genesis,
        }
    }

    pub fn settlement(&self) -> Option<&SettlementLayer> {
        match self {
            Self::Dev(spec) => spec.settlement.as_ref(),
            Self::Rollup(spec) => Some(&spec.settlement),
        }
    }

    pub fn fee_contracts(&self) -> &FeeContracts {
        match self {
            Self::Dev(spec) => &spec.fee_contracts,
            Self::Rollup(spec) => &spec.fee_contracts,
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

impl Default for ChainSpec {
    fn default() -> Self {
        Self::dev()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SettlementLayer {
    Ethereum {
        // The id of the settlement chain.
        id: eth::ChainId,

        // url for ethereum rpc provider
        rpc_url: Url,

        /// account on the ethereum network
        account: eth::Address,

        // - The core appchain contract used to settlement
        core_contract: eth::Address,

        // the block at which the core contract was deployed
        block: alloy_primitives::BlockNumber,
    },

    Starknet {
        // The id of the settlement chain.
        id: ChainId,

        // url for starknet rpc provider
        rpc_url: Url,

        /// account on the starknet network
        account: ContractAddress,

        // - The core appchain contract used to settlement
        core_contract: ContractAddress,

        // the block at which the core contract was deployed
        block: BlockNumber,
    },

    Sovereign {
        // Once Katana can sync from data availability layer, we can add the details of the data
        // availability layer to the chain spec for Katana to sync from it.
    },
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
