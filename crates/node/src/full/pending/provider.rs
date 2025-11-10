use std::fmt::Debug;

use katana_gateway_types::TxTryFromError;
use katana_primitives::transaction::{TxHash, TxNumber, TxWithHash};
use katana_provider::api::state::{StateFactoryProvider, StateProvider};
use katana_rpc_server::starknet::{PendingBlockProvider, StarknetApiResult};
use katana_rpc_types::RpcTxWithHash;

use crate::full::pending::PreconfStateFactory;

impl<P> PendingBlockProvider for PreconfStateFactory<P>
where
    P: StateFactoryProvider + Debug + 'static,
{
    fn get_pending_block_with_txs(
        &self,
    ) -> StarknetApiResult<Option<katana_rpc_types::PreConfirmedBlockWithTxs>> {
        if let Some(block) = self.block() {
            let transactions = block
                .transactions
                .clone()
                .into_iter()
                .map(|tx| Ok(RpcTxWithHash::from(TxWithHash::try_from(tx)?)))
                .collect::<Result<Vec<_>, TxTryFromError>>()
                .unwrap();

            Ok(Some(katana_rpc_types::PreConfirmedBlockWithTxs {
                transactions,
                block_number: 0,
                l1_da_mode: block.l1_da_mode,
                l1_gas_price: block.l1_gas_price,
                l2_gas_price: block.l2_gas_price,
                l1_data_gas_price: block.l1_data_gas_price,
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
                l1_gas_price: block.l1_gas_price,
                l2_gas_price: block.l2_gas_price,
                l1_data_gas_price: block.l1_data_gas_price,
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
                l1_gas_price: block.l1_gas_price,
                l2_gas_price: block.l2_gas_price,
                l1_data_gas_price: block.l1_data_gas_price,
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
        index: TxNumber,
    ) -> StarknetApiResult<Option<katana_rpc_types::RpcTxWithHash>> {
        Ok(None)
    }

    fn pending_state(&self) -> StarknetApiResult<Option<Box<dyn StateProvider>>> {
        Ok(Some(Box::new(self.state())))
    }

    fn get_pending_trace(
        &self,
        hash: TxHash,
    ) -> StarknetApiResult<Option<katana_rpc_types::TxTrace>> {
        Ok(None)
    }
}
