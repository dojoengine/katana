use katana_genesis::Genesis;
use katana_primitives::block::{ExecutableBlock, GasPrices, PartialHeader};
use katana_primitives::chain::ChainId;
use katana_primitives::da::L1DataAvailabilityMode;
use katana_primitives::state::StateUpdatesWithClasses;
use katana_primitives::version::CURRENT_STARKNET_VERSION;

pub mod file;
pub mod utils;

pub use file::*;
pub use utils::DEFAULT_APPCHAIN_FEE_TOKEN_ADDRESS;

use crate::{ChainSpecT, FeeContracts, SettlementLayer};

/// The rollup chain specification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainSpec {
    /// The rollup network chain id.
    pub id: ChainId,

    /// The chain's genesis states.
    pub genesis: Genesis,

    /// The chain fee token contract.
    pub fee_contracts: FeeContracts,

    /// The chain's settlement layer configurations.
    ///
    /// This is optional to support development mode where no settlement layer is configured.
    pub settlement: Option<SettlementLayer>,
}

//////////////////////////////////////////////////////////////
// 	ChainSpec implementations
//////////////////////////////////////////////////////////////

impl ChainSpec {
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

        let transactions = utils::GenesisTransactionsBuilder::new(self).build();

        ExecutableBlock { header, body: transactions }
    }

    pub fn state_updates(&self) -> StateUpdatesWithClasses {
        // Rollup chain spec state updates are derived from transaction execution,
        // so we return an empty state updates here.
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
