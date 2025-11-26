use katana_gateway_types::TxTryFromError;
use katana_primitives::block::FinalityStatus;
use katana_primitives::fee::PriceUnit;
use katana_primitives::transaction::{TxHash, TxNumber, TxType, TxWithHash};
use katana_primitives::Felt;
use katana_provider::api::state::StateProvider;
use katana_rpc_server::starknet::{PendingBlockProvider, StarknetApiResult};
use katana_rpc_types::{
    PreConfirmedStateUpdate, ReceiptBlockInfo, RpcTxReceipt, RpcTxWithHash, TxReceiptWithBlockInfo,
};

use crate::full::pending::PreconfStateFactory;

impl PendingBlockProvider for PreconfStateFactory {
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
        if let Some(preconf_block) = self.block() {
            let receipt = preconf_block
                .transaction_receipts
                .iter()
                .zip(preconf_block.transactions)
                .filter_map(|(receipt, tx)| {
                    if let Some(receipt) = receipt {
                        Some((receipt.clone(), tx.transaction.r#type()))
                    } else {
                        None
                    }
                })
                .find(|(receipt, ..)| receipt.transaction_hash == hash);

            let Some((receipt, r#type)) = receipt else { return Ok(None) };

            let status = FinalityStatus::PreConfirmed;
            let transaction_hash = receipt.transaction_hash;
            let block = ReceiptBlockInfo::PreConfirmed { block_number: 0 };

            let receipt = match r#type {
                TxType::Invoke => {
                    RpcTxReceipt::Invoke(receipt.to_rpc_invoke_receipt(status, PriceUnit::Fri))
                }

                TxType::Declare => {
                    RpcTxReceipt::Declare(receipt.to_rpc_declare_receipt(status, PriceUnit::Fri))
                }

                TxType::Deploy => RpcTxReceipt::Deploy(receipt.to_rpc_deploy_receipt(
                    status,
                    PriceUnit::Fri,
                    Default::default(),
                )),

                TxType::L1Handler => RpcTxReceipt::L1Handler(receipt.to_rpc_l1_handler_receipt(
                    status,
                    PriceUnit::Fri,
                    Default::default(),
                )),

                TxType::DeployAccount => {
                    RpcTxReceipt::DeployAccount(receipt.to_rpc_deploy_account_receipt(
                        status,
                        PriceUnit::Fri,
                        Default::default(),
                    ))
                }
            };

            Ok(Some(TxReceiptWithBlockInfo { transaction_hash, receipt, block }))
        } else {
            Ok(None)
        }
    }

    fn get_pending_state_update(
        &self,
    ) -> StarknetApiResult<Option<katana_rpc_types::PreConfirmedStateUpdate>> {
        if let Some(state_diff) = self.state_updates() {
            Ok(Some(PreConfirmedStateUpdate {
                old_root: Felt::ZERO,
                state_diff: state_diff.into(),
            }))
        } else {
            Ok(None)
        }
    }

    fn get_pending_transaction(
        &self,
        hash: TxHash,
    ) -> StarknetApiResult<Option<katana_rpc_types::RpcTxWithHash>> {
        if let Some(preconf_transactions) = self.transactions() {
            let transaction = preconf_transactions
                .iter()
                .find(|tx| tx.transaction_hash == hash)
                .cloned()
                .map(RpcTxWithHash::from);

            Ok(transaction)
        } else {
            Ok(None)
        }
    }

    fn get_pending_transaction_by_index(
        &self,
        index: TxNumber,
    ) -> StarknetApiResult<Option<katana_rpc_types::RpcTxWithHash>> {
        if let Some(preconf_transactions) = self.transactions() {
            Ok(preconf_transactions.get(index as usize).cloned().map(RpcTxWithHash::from))
        } else {
            Ok(None)
        }
    }

    fn pending_state(&self) -> StarknetApiResult<Option<Box<dyn StateProvider>>> {
        Ok(Some(Box::new(self.state())))
    }

    fn get_pending_trace(
        &self,
        hash: TxHash,
    ) -> StarknetApiResult<Option<katana_rpc_types::TxTrace>> {
        let _ = hash;
        Ok(None)
    }
}
