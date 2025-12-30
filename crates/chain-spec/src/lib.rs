use katana_genesis::Genesis;
use katana_primitives::block::{BlockHashOrNumber, BlockNumber, ExecutableBlock};
use katana_primitives::chain::ChainId;
use katana_primitives::state::StateUpdatesWithClasses;
use katana_primitives::{eth, ContractAddress};
use katana_rpc_types::GetBlockWithTxHashesResponse;
use serde::{Deserialize, Serialize};
use url::Url;

pub mod dev;
pub mod full_node;
pub mod rollup;

#[auto_impl::auto_impl(Arc, Box, &)]
pub trait ChainSpecT: Send + Sync + Clone + 'static {
    fn id(&self) -> ChainId;

    fn genesis(&self) -> &Genesis;

    fn fee_contracts(&self) -> &FeeContracts;

    fn settlement(&self) -> Option<&SettlementLayer>;

    fn block(&self) -> ExecutableBlock;

    fn state_updates(&self) -> StateUpdatesWithClasses;

    fn is_dev(&self) -> bool {
        false
    }
}

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

    pub fn settlement(&self) -> Option<&SettlementLayer> {
        match self {
            Self::Dev(spec) => spec.settlement.as_ref(),
            Self::Rollup(spec) => spec.settlement.as_ref(),
            Self::FullNode(spec) => spec.settlement.as_ref(),
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

impl ChainSpecT for ChainSpec {
    fn id(&self) -> ChainId {
        match self {
            Self::Dev(spec) => spec.id,
            Self::Rollup(spec) => spec.id,
            Self::FullNode(spec) => spec.id,
        }
    }

    fn genesis(&self) -> &Genesis {
        match self {
            Self::Dev(spec) => &spec.genesis,
            Self::Rollup(spec) => &spec.genesis,
            Self::FullNode(spec) => &spec.genesis,
        }
    }

    fn fee_contracts(&self) -> &FeeContracts {
        match self {
            Self::Dev(spec) => &spec.fee_contracts,
            Self::Rollup(spec) => &spec.fee_contracts,
            Self::FullNode(spec) => &spec.fee_contracts,
        }
    }

    fn settlement(&self) -> Option<&SettlementLayer> {
        match self {
            Self::Dev(spec) => spec.settlement.as_ref(),
            Self::Rollup(spec) => spec.settlement.as_ref(),
            Self::FullNode(spec) => spec.settlement.as_ref(),
        }
    }

    fn block(&self) -> ExecutableBlock {
        match self {
            Self::Dev(spec) => spec.block(),
            Self::Rollup(spec) => spec.block(),
            Self::FullNode(spec) => spec.block(),
        }
    }

    fn state_updates(&self) -> StateUpdatesWithClasses {
        match self {
            Self::Dev(spec) => spec.state_updates(),
            Self::Rollup(spec) => spec.state_updates(),
            Self::FullNode(spec) => spec.state_updates(),
        }
    }

    fn is_dev(&self) -> bool {
        matches!(self, Self::Dev(_))
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

pub struct ForkedChainSpec {}

impl ForkedChainSpec {
    pub async fn new(
        client: katana_rpc_client::starknet::Client,
        block: Option<BlockHashOrNumber>,
    ) -> Self {
        let chain_id = client.chain_id().await.unwrap();

        // If the fork block number is not specified, we use the latest accepted block on the forked
        // network.
        let block_id = if let Some(id) = block {
            id
        } else {
            let res = client.block_number().await.unwrap();
            BlockHashOrNumber::Num(res.block_number)
        };

        let block = client.get_block_with_tx_hashes(block_id.into()).await.unwrap();

        let GetBlockWithTxHashesResponse::Block(forked_block) = block else {
            panic!("forking a pending block is not allowed")
        };

        Self {}
    }
}
