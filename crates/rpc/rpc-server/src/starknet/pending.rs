use std::fmt::Debug;

use katana_core::service::block_producer::{BlockProducer, BlockProducerMode};
use katana_executor::ExecutorFactory;
use katana_primitives::block::{BlockIdOrTag, PartialHeader};
use katana_primitives::da::L1DataAvailabilityMode;
use katana_primitives::execution::TypedTransactionExecutionInfo;
use katana_primitives::transaction::{TxHash, TxNumber};
use katana_primitives::version::CURRENT_STARKNET_VERSION;
use katana_provider::api::block::BlockNumberProvider;
use katana_provider::api::state::{StateFactoryProvider, StateProvider};
use katana_provider::providers::db::cached::CachedStateProvider;
use katana_rpc_client::starknet::Client;
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

/// A pending block provider that checks the optimistic state for transactions/receipts,
/// then falls back to the client for all queries.
#[derive(Debug, Clone)]
pub struct OptimisticPendingBlockProvider {
    optimistic_state: katana_optimistic::executor::OptimisticState,
    client: Client,
    storage: katana_core::backend::storage::Blockchain,
}

impl OptimisticPendingBlockProvider {
    pub fn new(
        optimistic_state: katana_optimistic::executor::OptimisticState,
        client: Client,
        provider: katana_core::backend::storage::Blockchain,
    ) -> Self {
        Self { optimistic_state, client, storage: provider }
    }
}

impl PendingBlockProvider for OptimisticPendingBlockProvider {
    fn pending_state(&self) -> StarknetApiResult<Option<Box<dyn StateProvider>>> {
        let latest_state = self.storage.provider().latest()?;
        Ok(Some(self.optimistic_state.get_optimistic_state(latest_state)))
    }

    fn get_pending_state_update(&self) -> StarknetApiResult<Option<PreConfirmedStateUpdate>> {
        self.client.get_pending_state_update()
    }

    fn get_pending_block_with_txs(&self) -> StarknetApiResult<Option<PreConfirmedBlockWithTxs>> {
        self.client.get_pending_block_with_txs()
    }

    fn get_pending_block_with_receipts(
        &self,
    ) -> StarknetApiResult<Option<PreConfirmedBlockWithReceipts>> {
        self.client.get_pending_block_with_receipts()
    }

    fn get_pending_block_with_tx_hashes(
        &self,
    ) -> StarknetApiResult<Option<PreConfirmedBlockWithTxHashes>> {
        self.client.get_pending_block_with_tx_hashes()
    }

    fn get_pending_transaction(&self, hash: TxHash) -> StarknetApiResult<Option<RpcTxWithHash>> {
        // First, check optimistic state
        let transactions = self.optimistic_state.transactions.read();
        if let Some((tx, _result)) = transactions.iter().find(|(tx, _)| tx.hash == hash) {
            return Ok(Some(RpcTxWithHash::from(tx.clone())));
        }

        // Fall back to client
        self.client.get_pending_transaction(hash)
    }

    fn get_pending_receipt(
        &self,
        hash: TxHash,
    ) -> StarknetApiResult<Option<TxReceiptWithBlockInfo>> {
        // First, check optimistic state
        let transactions = self.optimistic_state.transactions.read();
        if let Some((_tx, result)) = transactions.iter().find(|(tx, _)| tx.hash == hash) {
            if let katana_executor::ExecutionResult::Success { receipt, .. } = result {
                // Get the latest block number to use as reference
                let latest_num = self.storage.provider().latest_number().map_err(|e| {
                    crate::starknet::StarknetApiError::unexpected(format!(
                        "Failed to get latest block number: {e}"
                    ))
                })?;

                // Create block info as PreConfirmed (optimistic tx not yet in a block)
                let block = ReceiptBlockInfo::PreConfirmed { block_number: latest_num + 1 };

                // Create receipt with block info
                let receipt_with_block = TxReceiptWithBlockInfo::new(
                    block,
                    hash,
                    FinalityStatus::PreConfirmed,
                    receipt.clone(),
                );

                return Ok(Some(receipt_with_block));
            }
        }

        // Fall back to client
        self.client.get_pending_receipt(hash)
    }

    fn get_pending_trace(&self, hash: TxHash) -> StarknetApiResult<Option<TxTrace>> {
        // First, check optimistic state
        let transactions = self.optimistic_state.transactions.read();
        if let Some((tx, result)) = transactions.iter().find(|(tx, _)| tx.hash == hash) {
            if let katana_executor::ExecutionResult::Success { trace, .. } = result {
                let typed_trace = TypedTransactionExecutionInfo::new(tx.r#type(), trace.clone());
                return Ok(Some(TxTrace::from(typed_trace)));
            }
        }

        // Fall back to client
        self.client.get_pending_trace(hash)
    }

    fn get_pending_transaction_by_index(
        &self,
        index: TxNumber,
    ) -> StarknetApiResult<Option<RpcTxWithHash>> {
        // Check optimistic state by index
        let transactions = self.optimistic_state.transactions.read();
        if let Some((tx, _result)) = transactions.get(index as usize) {
            return Ok(Some(RpcTxWithHash::from(tx.clone())));
        }

        // Fall back to client
        self.client.get_pending_transaction_by_index(index)
    }
}

impl PendingBlockProvider for katana_rpc_client::starknet::Client {
    fn get_pending_state_update(&self) -> StarknetApiResult<Option<PreConfirmedStateUpdate>> {
        let result = futures::executor::block_on(async {
            self.get_state_update(BlockIdOrTag::PreConfirmed).await
        });

        match result {
            Ok(state_update) => match state_update {
                katana_rpc_types::state_update::StateUpdate::PreConfirmed(update) => {
                    Ok(Some(update))
                }
                _ => Ok(None),
            },
            Err(_) => Ok(None),
        }
    }

    fn get_pending_block_with_txs(&self) -> StarknetApiResult<Option<PreConfirmedBlockWithTxs>> {
        let result = futures::executor::block_on(async {
            self.get_block_with_txs(BlockIdOrTag::PreConfirmed).await
        });

        match result {
            Ok(block) => match block {
                katana_rpc_types::block::MaybePreConfirmedBlock::PreConfirmed(block) => {
                    Ok(Some(block))
                }
                _ => Ok(None),
            },
            Err(_) => Ok(None),
        }
    }

    fn get_pending_block_with_receipts(
        &self,
    ) -> StarknetApiResult<Option<PreConfirmedBlockWithReceipts>> {
        let result = futures::executor::block_on(async {
            self.get_block_with_receipts(BlockIdOrTag::PreConfirmed).await
        });

        match result {
            Ok(block) => match block {
                katana_rpc_types::block::GetBlockWithReceiptsResponse::PreConfirmed(block) => {
                    Ok(Some(block))
                }
                _ => Ok(None),
            },
            Err(_) => Ok(None),
        }
    }

    fn get_pending_block_with_tx_hashes(
        &self,
    ) -> StarknetApiResult<Option<PreConfirmedBlockWithTxHashes>> {
        let result = futures::executor::block_on(async {
            self.get_block_with_tx_hashes(BlockIdOrTag::PreConfirmed).await
        });

        match result {
            Ok(block) => match block {
                katana_rpc_types::block::GetBlockWithTxHashesResponse::PreConfirmed(block) => {
                    Ok(Some(block))
                }
                _ => Ok(None),
            },
            Err(_) => Ok(None),
        }
    }

    fn get_pending_transaction(&self, hash: TxHash) -> StarknetApiResult<Option<RpcTxWithHash>> {
        let result =
            futures::executor::block_on(async { self.get_transaction_by_hash(hash).await });

        match result {
            Ok(tx) => Ok(Some(tx)),
            Err(_) => Ok(None),
        }
    }

    fn get_pending_receipt(
        &self,
        hash: TxHash,
    ) -> StarknetApiResult<Option<TxReceiptWithBlockInfo>> {
        let result =
            futures::executor::block_on(async { self.get_transaction_receipt(hash).await });

        match result {
            Ok(receipt) => Ok(Some(receipt)),
            Err(_) => Ok(None),
        }
    }

    fn get_pending_trace(&self, hash: TxHash) -> StarknetApiResult<Option<TxTrace>> {
        let result = futures::executor::block_on(async { self.trace_transaction(hash).await });

        match result {
            Ok(trace) => Ok(Some(trace)),
            Err(_) => Ok(None),
        }
    }

    fn get_pending_transaction_by_index(
        &self,
        index: TxNumber,
    ) -> StarknetApiResult<Option<RpcTxWithHash>> {
        let result = futures::executor::block_on(async {
            self.get_transaction_by_block_id_and_index(BlockIdOrTag::PreConfirmed, index).await
        });

        match result {
            Ok(tx) => Ok(Some(tx)),
            Err(_) => Ok(None),
        }
    }

    fn pending_state(&self) -> StarknetApiResult<Option<Box<dyn StateProvider>>> {
        // Client-based pending block provider doesn't provide state access
        Ok(None)
    }
}
