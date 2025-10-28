use std::fmt::Debug;

use katana_gateway_types::TxTryFromError;
use katana_primitives::block::{GasPrices, PartialHeader, PendingBlock};
use katana_primitives::transaction::{TxHash, TxWithHash};
use katana_primitives::Felt;
use katana_provider::api::state::{StateFactoryProvider, StateProvider};
use katana_rpc::starknet::{PendingBlockProvider, StarknetApiResult};
use katana_rpc_types::PreConfirmedBlockWithTxs;
use num_traits::ToPrimitive;
use starknet::core::types::ResourcePrice;

use crate::full::pending::PreconfStateFactory;

impl<P: StateFactoryProvider + Debug> PendingBlockProvider for PreconfStateFactory<P> {
    fn get_pending_block_with_txs(
        &self,
    ) -> StarknetApiResult<Option<katana_rpc_types::PreConfirmedBlockWithTxs>> {
        if let Some(block) = self.block() {
            let transactions = block
                .transactions
                .clone()
                .into_iter()
                .map(TxWithHash::try_from)
                .collect::<Result<Vec<_>, TxTryFromError>>()
                .unwrap();

            Ok(Some(katana_rpc_types::PreConfirmedBlockWithTxs {
                transactions,
                block_number: 0,
                l1_da_mode: block.l1_da_mode,
                l1_gas_price: to_gas_prices(block.l1_gas_price),
                l2_gas_price: to_gas_prices(block.l2_gas_price),
                l1_data_gas_price: to_gas_prices(block.l1_data_gas_price),
                sequencer_address: block.sequencer_address,
                starknet_version: block.starknet_version,
                timestamp: block.timestamp,
            }))
        } else {
            Ok(None)
        }
    }

    fn get_pending_block_with_receipts(
        &self,
    ) -> StarknetApiResult<Option<katana_rpc_types::PreConfirmedBlockWithReceipts>> {
        if let Some(block) = self.block() {
            Ok(Some(katana_rpc_types::PreConfirmedBlockWithReceipts {
                transactions: Vec::new(),
                block_number: 0,
                l1_da_mode: block.l1_da_mode,
                l1_gas_price: to_gas_prices(block.l1_gas_price),
                l2_gas_price: to_gas_prices(block.l2_gas_price),
                l1_data_gas_price: to_gas_prices(block.l1_data_gas_price),
                sequencer_address: block.sequencer_address,
                starknet_version: block.starknet_version,
                timestamp: block.timestamp,
            }))
        } else {
            Ok(None)
        }
    }

    fn get_pending_block_with_tx_hashes(
        &self,
    ) -> StarknetApiResult<Option<katana_rpc_types::PreConfirmedBlockWithTxHashes>> {
        if let Some(block) = self.block() {
            let transactions = block
                .transactions
                .clone()
                .into_iter()
                .map(|tx| tx.transaction_hash)
                .collect::<Vec<TxHash>>();

            Ok(Some(katana_rpc_types::PreConfirmedBlockWithTxHashes {
                transactions,
                block_number: 0,
                l1_da_mode: block.l1_da_mode,
                l1_gas_price: to_gas_prices(block.l1_gas_price),
                l2_gas_price: to_gas_prices(block.l2_gas_price),
                l1_data_gas_price: to_gas_prices(block.l1_data_gas_price),
                sequencer_address: block.sequencer_address,
                starknet_version: block.starknet_version,
                timestamp: block.timestamp,
            }))
        } else {
            Ok(None)
        }
    }

    fn get_pending_receipt(
        &self,
        hash: TxHash,
    ) -> StarknetApiResult<Option<katana_rpc_types::TxReceiptWithBlockInfo>> {
        Ok(None)
    }

    fn get_pending_state_update(
        &self,
    ) -> StarknetApiResult<Option<katana_rpc_types::PreConfirmedStateUpdate>> {
        Ok(None)
    }

    fn get_pending_transaction(
        &self,
        hash: TxHash,
    ) -> StarknetApiResult<Option<katana_rpc_types::RpcTxWithHash>> {
        Ok(None)
    }

    fn get_pending_transaction_by_index(
        &self,
        hash: TxHash,
    ) -> StarknetApiResult<Option<katana_rpc_types::RpcTxWithHash>> {
        Ok(None)
    }

    fn pending_state(&self) -> StarknetApiResult<Option<Box<dyn StateProvider>>> {
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
