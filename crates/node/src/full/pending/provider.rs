use std::fmt::Debug;

use katana_gateway_types::TxTryFromError;
use katana_primitives::block::{GasPrices, PartialHeader, PendingBlock};
use katana_primitives::transaction::TxWithHash;
use katana_primitives::Felt;
use katana_provider::api::state::{StateFactoryProvider, StateProvider};
use katana_rpc::starknet::PendingBlockProvider;
use num_traits::ToPrimitive;
use starknet::core::types::ResourcePrice;

use crate::full::pending::PreconfStateFactory;

impl<P: StateFactoryProvider + Debug> PendingBlockProvider for PreconfStateFactory<P> {
    fn pending_block(&self) -> Option<PendingBlock> {
        if let Some(preconf_block) = self.block() {
            Some(PendingBlock {
                header: PartialHeader {
                    l1_da_mode: preconf_block.l1_da_mode,
                    l1_gas_prices: to_gas_prices(preconf_block.l1_gas_price),
                    l2_gas_prices: to_gas_prices(preconf_block.l2_gas_price),
                    l1_data_gas_prices: to_gas_prices(preconf_block.l1_data_gas_price),
                    sequencer_address: preconf_block.sequencer_address,
                    starknet_version: preconf_block.starknet_version.try_into().unwrap(),
                    timestamp: preconf_block.timestamp,
                    number: 0,
                    parent_hash: Felt::ZERO,
                },
                body: preconf_block
                    .transactions
                    .clone()
                    .into_iter()
                    .map(TxWithHash::try_from)
                    .collect::<Result<Vec<_>, TxTryFromError>>()
                    .unwrap(),
            })
        } else {
            None
        }
    }

    fn pending_state_update(&self) -> Option<katana_primitives::state::StateUpdates> {
        self.state_updates()
    }

    fn pending_transactions(
        &self,
    ) -> Vec<(
        katana_primitives::transaction::TxWithHash,
        Option<katana_primitives::receipt::Receipt>,
    )> {
        todo!()
    }

    fn pending_state(&self) -> Option<Box<dyn StateProvider>> {
        Some(Box::new(self.state()))
    }
}

fn to_gas_prices(prices: ResourcePrice) -> GasPrices {
    let eth = prices.price_in_wei.to_u128().expect("valid u128");
    let strk = prices.price_in_fri.to_u128().expect("valid u128");
    // older blocks might have zero gas prices (recent Starknet upgrade has made the minimum gas
    // prices to 1) we may need to handle this case if we want to be able to compute the
    // block hash correctly
    let eth = if eth == 0 { 1 } else { eth };
    let strk = if strk == 0 { 1 } else { strk };
    unsafe { GasPrices::new_unchecked(eth, strk) }
}
