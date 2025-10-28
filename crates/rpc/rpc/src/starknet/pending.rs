use std::fmt::Debug;

use katana_core::service::block_producer::{BlockProducer, BlockProducerMode};
use katana_executor::ExecutorFactory;
use katana_primitives::block::PartialHeader;
use katana_primitives::da::L1DataAvailabilityMode;
use katana_primitives::execution::TypedTransactionExecutionInfo;
use katana_primitives::transaction::{TxHash, TxNumber};
use katana_primitives::version::CURRENT_STARKNET_VERSION;
use katana_primitives::{block::PartialHeader, transaction::TxNumber};
use katana_provider::api::state::StateProvider;
use katana_rpc_types::{
    FinalityStatus, PreConfirmedBlockWithReceipts, PreConfirmedBlockWithTxHashes,
    PreConfirmedBlockWithTxs, PreConfirmedStateUpdate, ReceiptBlockInfo, RpcTxWithHash,
    TxReceiptWithBlockInfo, TxTrace,
};

use crate::starknet::StarknetApiResult;

#[auto_impl::auto_impl(Box)]
pub trait PendingBlockProvider: Debug + Send + Sync + 'static {
    fn pending_state(&self) -> StarknetApiResult<Option<Box<dyn StateProvider>>>;

    fn get_pending_state_update(&self) -> StarknetApiResult<Option<PreConfirmedStateUpdate>>;

    fn get_pending_block_with_txs(&self) -> StarknetApiResult<Option<PreConfirmedBlockWithTxs>>;

    fn get_pending_block_with_receipts(
        &self,
    ) -> StarknetApiResult<Option<PreConfirmedBlockWithReceipts>>;

    fn get_pending_block_with_tx_hashes(
        &self,
    ) -> StarknetApiResult<Option<PreConfirmedBlockWithTxHashes>>;

    fn get_pending_transaction(&self, hash: TxHash) -> StarknetApiResult<Option<RpcTxWithHash>>;

    fn get_pending_receipt(
        &self,
        hash: TxHash,
    ) -> StarknetApiResult<Option<TxReceiptWithBlockInfo>>;

    fn get_pending_trace(&self, hash: TxHash) -> StarknetApiResult<Option<TxTrace>>;

    fn get_pending_transaction_by_index(
        &self,
        index: TxNumber,
    ) -> StarknetApiResult<Option<RpcTxWithHash>>;
}

impl<EF: ExecutorFactory> PendingBlockProvider for BlockProducer<EF> {
    fn pending_state(&self) -> StarknetApiResult<Option<Box<dyn StateProvider>>> {
        match &*self.producer.read() {
            BlockProducerMode::Instant(_) => Ok(None),
            BlockProducerMode::Interval(producer) => Ok(Some(producer.executor().read().state())),
        }
    }

    fn get_pending_transaction(&self, hash: TxHash) -> StarknetApiResult<Option<RpcTxWithHash>> {
        match &*self.producer.read() {
            BlockProducerMode::Instant(_) => Ok(None),
            BlockProducerMode::Interval(producer) => {
                let result = producer
                    .executor()
                    .read()
                    .transactions()
                    .iter()
                    .find(|(tx, ..)| tx.hash == hash)
                    .map(|(tx, ..)| RpcTxWithHash::from(tx.clone()));

                Ok(result)
            }
        }
    }

    fn get_pending_block_with_txs(&self) -> StarknetApiResult<Option<PreConfirmedBlockWithTxs>> {
        match &*self.producer.read() {
            BlockProducerMode::Instant(_) => Ok(None),
            BlockProducerMode::Interval(producer) => {
                let block_env = producer.executor().read().block_env();

                let l2_gas_prices = block_env.l2_gas_prices.clone();
                let l1_gas_prices = block_env.l1_gas_prices.clone();
                let l1_data_gas_prices = block_env.l1_data_gas_prices.clone();

                let header = PartialHeader {
                    l1_da_mode: L1DataAvailabilityMode::Calldata,
                    l2_gas_prices,
                    l1_gas_prices,
                    l1_data_gas_prices,
                    number: block_env.number,
                    parent_hash: Default::default(),
                    timestamp: block_env.timestamp,
                    sequencer_address: block_env.sequencer_address,
                    starknet_version: CURRENT_STARKNET_VERSION,
                };

                // A block should only include successful transactions, we filter out the
                // failed ones (didn't pass validation stage).
                let body = producer
                    .executor()
                    .read()
                    .transactions()
                    .iter()
                    .filter(|(_, receipt)| receipt.is_success())
                    .map(|(tx, _)| tx.clone())
                    .collect::<Vec<_>>();

                Ok(Some(PreConfirmedBlockWithTxs::new(header, body)))
            }
        }
    }

    fn get_pending_block_with_receipts(
        &self,
    ) -> StarknetApiResult<Option<PreConfirmedBlockWithReceipts>> {
        match &*self.producer.read() {
            BlockProducerMode::Instant(_) => Ok(None),
            BlockProducerMode::Interval(producer) => {
                let block_env = producer.executor().read().block_env();

                let l2_gas_prices = block_env.l2_gas_prices.clone();
                let l1_gas_prices = block_env.l1_gas_prices.clone();
                let l1_data_gas_prices = block_env.l1_data_gas_prices.clone();

                let header = PartialHeader {
                    l1_da_mode: L1DataAvailabilityMode::Calldata,
                    l2_gas_prices,
                    l1_gas_prices,
                    l1_data_gas_prices,
                    number: block_env.number,
                    parent_hash: Default::default(),
                    timestamp: block_env.timestamp,
                    sequencer_address: block_env.sequencer_address,
                    starknet_version: CURRENT_STARKNET_VERSION,
                };

                let executor = producer.executor();
                let lock = executor.read();

                let body = lock
                    .transactions()
                    .iter()
                    .filter(|(_, receipt)| receipt.is_success())
                    .map(|(tx, receipt)| (tx.clone(), receipt.receipt().cloned().unwrap()));

                Ok(Some(PreConfirmedBlockWithReceipts::new(header, body)))
            }
        }
    }

    fn get_pending_block_with_tx_hashes(
        &self,
    ) -> StarknetApiResult<Option<PreConfirmedBlockWithTxHashes>> {
        match &*self.producer.read() {
            BlockProducerMode::Instant(_) => Ok(None),
            BlockProducerMode::Interval(producer) => {
                let block_env = producer.executor().read().block_env();

                let l2_gas_prices = block_env.l2_gas_prices.clone();
                let l1_gas_prices = block_env.l1_gas_prices.clone();
                let l1_data_gas_prices = block_env.l1_data_gas_prices.clone();

                let header = PartialHeader {
                    l1_da_mode: L1DataAvailabilityMode::Calldata,
                    l2_gas_prices,
                    l1_gas_prices,
                    l1_data_gas_prices,
                    number: block_env.number,
                    parent_hash: Default::default(),
                    timestamp: block_env.timestamp,
                    sequencer_address: block_env.sequencer_address,
                    starknet_version: CURRENT_STARKNET_VERSION,
                };

                let body = producer
                    .executor()
                    .read()
                    .transactions()
                    .iter()
                    .filter(|(_, receipt)| receipt.is_success())
                    .map(|(tx, _)| tx.hash)
                    .collect::<Vec<TxHash>>();

                Ok(Some(PreConfirmedBlockWithTxHashes::new(header, body)))
            }
        }
    }

    fn get_pending_receipt(
        &self,
        hash: TxHash,
    ) -> StarknetApiResult<Option<TxReceiptWithBlockInfo>> {
        match &*self.producer.read() {
            BlockProducerMode::Instant(_) => Ok(None),
            BlockProducerMode::Interval(producer) => {
                let executor = producer.executor();
                let executor_lock = executor.read();

                let result = executor_lock.transactions().iter().find_map(|(tx, receipt)| {
                    if tx.hash == hash {
                        receipt.receipt().cloned()
                    } else {
                        None
                    }
                });

                if let Some(receipt) = result {
                    let pending_block_env = executor_lock.block_env();
                    let pending_block_number = pending_block_env.number;

                    let receipt = TxReceiptWithBlockInfo::new(
                        ReceiptBlockInfo::PreConfirmed { block_number: pending_block_number },
                        hash,
                        FinalityStatus::AcceptedOnL2,
                        receipt,
                    );

                    StarknetApiResult::Ok(Some(receipt))
                } else {
                    StarknetApiResult::Ok(None)
                }
            }
        }
    }

    fn get_pending_trace(&self, hash: TxHash) -> StarknetApiResult<Option<TxTrace>> {
        match &*self.producer.read() {
            BlockProducerMode::Instant(_) => Ok(None),
            BlockProducerMode::Interval(producer) => {
                let executor = producer.executor();
                let executor_lock = executor.read();

                let result = executor_lock.transactions().iter().find(|(t, _)| t.hash == hash);

                if let Some((tx, res)) = result {
                    if let Some(trace) = res.trace() {
                        let trace = TypedTransactionExecutionInfo::new(tx.r#type(), trace.clone());
                        return Ok(Some(TxTrace::from(trace)));
                    }
                }

                Ok(None)
            }
        }
    }

    fn get_pending_state_update(&self) -> StarknetApiResult<Option<PreConfirmedStateUpdate>> {
        Ok(None)
    }

    fn get_pending_transaction_by_index(
        &self,
        index: TxNumber,
    ) -> StarknetApiResult<Option<RpcTxWithHash>> {
        match &*self.producer.read() {
            BlockProducerMode::Instant(_) => Ok(None),
            BlockProducerMode::Interval(producer) => {
                let result = producer
                    .executor()
                    .read()
                    .transactions()
                    .get(index as usize)
                    .map(|(tx, ..)| tx.clone())
                    .map(RpcTxWithHash::from);

                if let tx @ Some(..) = result {
                    StarknetApiResult::Ok(tx)
                } else {
                    StarknetApiResult::Ok(None)
                }
            }
        }
    }
}
