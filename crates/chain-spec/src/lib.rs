use katana_genesis::Genesis;
use katana_primitives::block::BlockNumber;
use katana_primitives::chain::ChainId;
use katana_primitives::{eth, ContractAddress};
use serde::{Deserialize, Serialize};
use url::Url;

pub mod dev;
pub mod full_node;
pub mod rollup;
pub mod settlement_check;
pub mod tee;

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
            Self::Rollup(spec) => Some(&spec.settlement),
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

        // - The core appchain contract used to settlement
        core_contract: ContractAddress,

        // the block at which the core contract was deployed
        block: BlockNumber,

        /// The proof system the core contract was initialized for. Determines which fields of
        /// Piltover's `program_info` are meaningful and validated at startup.
        #[serde(default)]
        proof_kind: SettlementProofKind,
    },

    Sovereign {
        // Once Katana can sync from data availability layer, we can add the details of the data
        // availability layer to the chain spec for Katana to sync from it.
    },
}

/// The proof system a Starknet settlement contract was initialized for.
///
/// Validity-proof chains have meaningful program hashes (SNOS, layout-bridge, bootloader) on the
/// core contract. TEE chains do not — only the SNOS config hash is validated against the chain's
/// own id and fee token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SettlementProofKind {
    #[default]
    ValidityProof,
    Tee,
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
