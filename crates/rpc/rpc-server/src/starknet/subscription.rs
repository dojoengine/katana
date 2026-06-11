use jsonrpsee::core::SubscriptionResult;
use jsonrpsee::types::ErrorObjectOwned;
use jsonrpsee::PendingSubscriptionSink;
use katana_pool::api::TransactionPool;
use katana_primitives::block::{BlockHashOrNumber, BlockIdOrTag, BlockNumber, FinalityStatus};
use katana_primitives::receipt::Receipt;
use katana_primitives::transaction::TxHash;
use katana_primitives::ContractAddress;
use katana_provider::api::block::{BlockHashProvider, BlockNumberProvider, HeaderProvider};
use katana_provider::api::transaction::{
    ReceiptProvider, TransactionProvider, TransactionStatusProvider,
};
use katana_provider::{ProviderFactory, ProviderRO};
use katana_rpc_api::error::starknet::StarknetApiError;
use katana_rpc_api::starknet::StarknetSubscriptionApiServer;
use katana_rpc_types::event::{EmittedEvent, RawEvent};
use katana_rpc_types::receipt::{ReceiptBlockInfo, TxReceiptWithBlockInfo};
use katana_rpc_types::subscription::{
    EmittedEventWithFinalityStatus, ExecutionStatus, NewTxnStatus, SubscriptionBlockHeader,
    TransactionStatusUpdate, TxWithFinalityStatus,
};
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;
use tracing::warn;

use super::{PendingBlockProvider, StarknetApi};
use crate::utils::events::Filter;

const MAX_BACKFILL_BLOCKS: u64 = 1024;

/// Helper to send a serializable value as a subscription message.
async fn send<T: jsonrpsee::core::Serialize>(
    sink: &jsonrpsee::SubscriptionSink,
    value: &T,
) -> Result<(), jsonrpsee::core::server::DisconnectError> {
    let msg = serde_json::value::to_raw_value(value).unwrap();
    sink.send(jsonrpsee::SubscriptionMessage::from(msg)).await
}

#[jsonrpsee::core::async_trait]
impl<Pool, PP, PF> StarknetSubscriptionApiServer for StarknetApi<Pool, PP, PF>
where
    Pool: TransactionPool + 'static,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO,
{
    async fn subscribe_new_heads(
        &self,
        pending: PendingSubscriptionSink,
        block_id: Option<BlockIdOrTag>,
    ) -> SubscriptionResult {
        let backfill_start = if let Some(ref block_id) = block_id {
            let provider = self.storage().provider();
            match validate_backfill(&provider, *block_id) {
                Ok(range) => range,
                Err(err) => {
                    pending.reject(err).await;
                    return Ok(());
                }
            }
        } else {
            None
        };

        let sink = pending.accept().await?;
        let mut rx = self.subscribe_blocks();

        // Backfill historical headers.
        if let Some(from) = backfill_start {
            let provider = self.storage().provider();
            let latest = provider
                .latest_number()
                .map_err(|e| StarknetApiError::unexpected(e).into_subscription_error())?;

            for num in from..=latest {
                if sink.is_closed() {
                    return Ok(());
                }

                let header =
                    build_block_header(&provider, num).map_err(|e| e.into_subscription_error())?;

                if let Some(header) = header {
                    if send(&sink, &header).await.is_err() {
                        return Ok(());
                    }
                }
            }
        }

        // Live stream.
        loop {
            tokio::select! {
                result = rx.recv() => {
                    match result {
                        Ok(outcome) => {
                            let provider = self.storage().provider();
                            match build_block_header(&provider, outcome.block_number) {
                                Ok(Some(header)) => {
                                    if send(&sink, &header).await.is_err() {
                                        return Ok(());
                                    }
                                }

                                Ok(None) => {}

                                Err(e) => return Err(e.into_subscription_error())
                            }
                        }

                        Err(RecvError::Lagged(n)) => {
                            warn!(target: "rpc::subscription", skipped = n, "Subscription lagged");
                        }

                        Err(RecvError::Closed) => return Ok(()),
                    }
                }

                _ = sink.closed() => return Ok(()),

            }
        }
    }

    async fn subscribe_events(
        &self,
        pending: PendingSubscriptionSink,
        from_address: Option<ContractAddress>,
        keys: Option<Vec<Vec<katana_primitives::Felt>>>,
        block_id: Option<BlockIdOrTag>,
    ) -> SubscriptionResult {
        let backfill_start = if let Some(ref block_id) = block_id {
            let provider = self.storage().provider();
            match validate_backfill(&provider, *block_id) {
                Ok(range) => Some(range),
                Err(err) => {
                    pending.reject(err).await;
                    return Ok(());
                }
            }
        } else {
            None
        };

        let sink = pending.accept().await?;
        let mut rx = self.subscribe_blocks();
        let filter = Filter { address: from_address, keys };

        // Backfill historical events.
        if let Some(Some(from)) = backfill_start {
            let provider = self.storage().provider();
            let latest = provider
                .latest_number()
                .map_err(|e| StarknetApiError::unexpected(e).into_subscription_error())?;

            for num in from..=latest {
                if sink.is_closed() {
                    return Ok(());
                }

                let events = fetch_block_events(&provider, num, &filter)
                    .map_err(|e| e.into_subscription_error())?;

                for event in events {
                    let notification = EmittedEventWithFinalityStatus {
                        event,
                        finality_status: FinalityStatus::AcceptedOnL2,
                    };
                    if send(&sink, &notification).await.is_err() {
                        return Ok(());
                    }
                }
            }
        }

        // Live stream.
        loop {
            tokio::select! {
                _ = sink.closed() => return Ok(()),
                result = rx.recv() => {
                    match result {
                        Ok(outcome) => {
                            let provider = self.storage().provider();
                            let events = fetch_block_events(&provider, outcome.block_number, &filter)
                                .map_err(|e| e.into_subscription_error())?;

                            for event in events {
                                let notification = EmittedEventWithFinalityStatus {
                                    event,
                                    finality_status: FinalityStatus::AcceptedOnL2,
                                };
                                if send(&sink, &notification).await.is_err() {
                                    return Ok(());
                                }
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            warn!(target: "rpc::subscription", skipped = n, "Subscription lagged");
                        }
                        Err(broadcast::error::RecvError::Closed) => return Ok(()),
                    }
                }
            }
        }
    }

    async fn subscribe_transaction_status(
        &self,
        pending: PendingSubscriptionSink,
        transaction_hash: TxHash,
    ) -> SubscriptionResult {
        let sink = pending.accept().await?;
        let mut rx = self.subscribe_blocks();
        let provider = self.storage().provider();

        // Check if already in storage (finalized).
        if let Ok(Some(finality_status)) = provider.transaction_status(transaction_hash) {
            let (execution_status, failure_reason) =
                match provider.receipt_by_hash(transaction_hash) {
                    Ok(Some(receipt)) => receipt_execution_status(&receipt),
                    _ => (Some(ExecutionStatus::Succeeded), None),
                };
            let update = TransactionStatusUpdate {
                transaction_hash,
                status: NewTxnStatus { finality_status, execution_status, failure_reason },
            };
            let _ = send(&sink, &update).await;
            return Ok(());
        }

        // Check if in pool (received).
        if self.pool().contains(transaction_hash) {
            let update = TransactionStatusUpdate {
                transaction_hash,
                status: NewTxnStatus {
                    finality_status: FinalityStatus::AcceptedOnL2,
                    execution_status: None,
                    failure_reason: None,
                },
            };
            if send(&sink, &update).await.is_err() {
                return Ok(());
            }
        }

        // Wait for the transaction to be included in a block.
        loop {
            tokio::select! {
                _ = sink.closed() => return Ok(()),
                result = rx.recv() => {
                    match result {
                        Ok(outcome) => {
                            if outcome.txs.contains(&transaction_hash) {
                                let provider = self.storage().provider();
                                let (execution_status, failure_reason) =
                                    match provider.receipt_by_hash(transaction_hash) {
                                        Ok(Some(receipt)) => receipt_execution_status(&receipt),
                                        _ => (Some(ExecutionStatus::Succeeded), None),
                                    };
                                let update = TransactionStatusUpdate {
                                    transaction_hash,
                                    status: NewTxnStatus {
                                        finality_status: FinalityStatus::AcceptedOnL2,
                                        execution_status,
                                        failure_reason,
                                    },
                                };
                                let _ = send(&sink, &update).await;
                                return Ok(());
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            warn!(target: "rpc::subscription", skipped = n, "Subscription lagged");
                        }
                        Err(broadcast::error::RecvError::Closed) => return Ok(()),
                    }
                }
            }
        }
    }

    async fn subscribe_new_transaction_receipts(
        &self,
        pending: PendingSubscriptionSink,
        sender_address: Option<Vec<ContractAddress>>,
    ) -> SubscriptionResult {
        let sink = pending.accept().await?;
        let mut rx = self.subscribe_blocks();

        loop {
            tokio::select! {
                _ = sink.closed() => return Ok(()),
                result = rx.recv() => {
                    match result {
                        Ok(outcome) => {
                            let provider = self.storage().provider();
                            let block_num = outcome.block_number;
                            let block_id = BlockHashOrNumber::Num(block_num);

                            let block_hash = provider
                                .block_hash_by_num(block_num)
                                .map_err(|e| StarknetApiError::from(e).into_subscription_error())?
                                .ok_or_else(|| StarknetApiError::BlockNotFound.into_subscription_error())?;

                            let txs = provider
                                .transactions_by_block(block_id)
                                .map_err(|e| StarknetApiError::from(e).into_subscription_error())?
                                .unwrap_or_default();

                            let receipts = provider
                                .receipts_by_block(block_id)
                                .map_err(|e| StarknetApiError::from(e).into_subscription_error())?
                                .unwrap_or_default();

                            for (tx, receipt) in txs.iter().zip(receipts.into_iter()) {
                                if let Some(ref addrs) = sender_address {
                                    if !tx_sender_address(&tx.transaction).is_some_and(|a| addrs.contains(&a)) {
                                        continue;
                                    }
                                }
                                let block_info = ReceiptBlockInfo::Block { block_hash, block_number: block_num };
                                let r = TxReceiptWithBlockInfo::new(
                                    block_info,
                                    tx.hash,
                                    FinalityStatus::AcceptedOnL2,
                                    receipt,
                                );
                                if send(&sink, &r).await.is_err() {
                                    return Ok(());
                                }
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            warn!(target: "rpc::subscription", skipped = n, "Subscription lagged");
                        }
                        Err(broadcast::error::RecvError::Closed) => return Ok(()),
                    }
                }
            }
        }
    }

    async fn subscribe_new_transactions(
        &self,
        pending: PendingSubscriptionSink,
        sender_address: Option<Vec<ContractAddress>>,
    ) -> SubscriptionResult {
        let sink = pending.accept().await?;
        let mut rx = self.subscribe_blocks();

        loop {
            tokio::select! {
                _ = sink.closed() => return Ok(()),
                result = rx.recv() => {
                    match result {
                        Ok(outcome) => {
                            let provider = self.storage().provider();
                            let block_num = outcome.block_number;
                            let block_id = BlockHashOrNumber::Num(block_num);

                            let txs = provider
                                .transactions_by_block(block_id)
                                .map_err(|e| StarknetApiError::from(e).into_subscription_error())?
                                .unwrap_or_default();

                            for tx in txs {
                                if let Some(ref addrs) = sender_address {
                                    if !tx_sender_address(&tx.transaction).is_some_and(|a| addrs.contains(&a)) {
                                        continue;
                                    }
                                }
                                let notification = TxWithFinalityStatus {
                                    transaction: tx.into(),
                                    finality_status: FinalityStatus::AcceptedOnL2,
                                };
                                if send(&sink, &notification).await.is_err() {
                                    return Ok(());
                                }
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            warn!(target: "rpc::subscription", skipped = n, "Subscription lagged");
                        }
                        Err(broadcast::error::RecvError::Closed) => return Ok(()),
                    }
                }
            }
        }
    }
}

// --- Helper functions ---

/// Validate backfill parameters and return the starting block number.
/// Returns `None` if block_id resolves to latest (no backfill needed).
/// Returns an `ErrorObjectOwned` suitable for `pending.reject()` on failure.
fn validate_backfill(
    provider: &impl ProviderRO,
    from: BlockIdOrTag,
) -> Result<Option<BlockNumber>, ErrorObjectOwned> {
    let latest_block_number =
        provider.latest_number().map_err(StarknetApiError::from).map_err(ErrorObjectOwned::from)?;

    let from_block = match from {
        BlockIdOrTag::Number(n) if n > latest_block_number => {
            return Err(StarknetApiError::BlockNotFound.into());
        }

        BlockIdOrTag::L1Accepted | BlockIdOrTag::PreConfirmed => {
            return Err(StarknetApiError::BlockNotFound.into());
        }

        BlockIdOrTag::Hash(hash) => {
            let result = provider.block_number_by_hash(hash).map_err(StarknetApiError::from)?;
            if let Some(block_number) = result {
                block_number
            } else {
                return Err(StarknetApiError::BlockNotFound.into());
            }
        }

        BlockIdOrTag::Number(number) => number,

        BlockIdOrTag::Latest => return Ok(None),
    };

    if latest_block_number.saturating_sub(from_block) > MAX_BACKFILL_BLOCKS {
        return Err(StarknetApiError::TooManyBlocksBack.into());
    }

    Ok(Some(from_block))
}

fn build_block_header(
    provider: &(impl HeaderProvider + BlockHashProvider),
    block_number: u64,
) -> Result<Option<SubscriptionBlockHeader>, StarknetApiError> {
    let header = match provider.header(BlockHashOrNumber::Num(block_number)) {
        Ok(Some(h)) => h,
        Ok(None) => return Ok(None),
        Err(e) => return Err(StarknetApiError::from(e)),
    };
    let block_hash = match provider.block_hash_by_num(block_number) {
        Ok(Some(h)) => h,
        Ok(None) => return Ok(None),
        Err(e) => return Err(StarknetApiError::from(e)),
    };
    Ok(Some(SubscriptionBlockHeader::new(block_hash, header)))
}

fn fetch_block_events(
    provider: &(impl BlockHashProvider + ReceiptProvider + TransactionProvider),
    block_number: u64,
    filter: &Filter,
) -> Result<Vec<EmittedEvent>, StarknetApiError> {
    let block_id = BlockHashOrNumber::Num(block_number);
    let block_hash = provider.block_hash_by_num(block_number).map_err(StarknetApiError::from)?;
    let receipts = provider.receipts_by_block(block_id).map_err(StarknetApiError::from)?;
    let txs = provider.transactions_by_block(block_id).map_err(StarknetApiError::from)?;

    let Some(block_hash) = block_hash else { return Ok(vec![]) };
    let Some(receipts) = receipts else { return Ok(vec![]) };
    let Some(txs) = txs else { return Ok(vec![]) };

    let mut events = Vec::new();
    for (tx_idx, (tx, receipt)) in txs.iter().zip(receipts.iter()).enumerate() {
        for (event_idx, event) in receipt.events().iter().enumerate() {
            if let Some(addr) = filter.address {
                if addr != event.from_address {
                    continue;
                }
            }
            if let Some(ref key_filters) = filter.keys {
                let matched = key_filters.iter().enumerate().all(|(i, keys)| {
                    keys.is_empty() || event.keys.get(i).is_some_and(|k| keys.contains(k))
                });
                if !matched {
                    continue;
                }
            }
            events.push(EmittedEvent {
                block_hash: Some(block_hash),
                block_number: Some(block_number),
                transaction_hash: tx.hash,
                transaction_index: tx_idx as u64,
                event_index: event_idx as u64,
                from_address: event.from_address,
                event: RawEvent { keys: event.keys.clone(), data: event.data.clone() },
            });
        }
    }
    Ok(events)
}

fn tx_sender_address(tx: &katana_primitives::transaction::Tx) -> Option<ContractAddress> {
    use katana_primitives::transaction::{DeclareTx, InvokeTx, Tx};
    match tx {
        Tx::Invoke(invoke) => match invoke {
            InvokeTx::V0(_) => None,
            InvokeTx::V1(tx) => Some(tx.sender_address),
            InvokeTx::V3(tx) => Some(tx.sender_address),
        },
        Tx::Declare(declare) => match declare {
            DeclareTx::V0(tx) => Some(tx.sender_address),
            DeclareTx::V1(tx) => Some(tx.sender_address),
            DeclareTx::V2(tx) => Some(tx.sender_address),
            DeclareTx::V3(tx) => Some(tx.sender_address),
        },
        Tx::DeployAccount(deploy) => Some(deploy.contract_address()),
        Tx::L1Handler(_) | Tx::Deploy(_) => None,
    }
}

fn receipt_execution_status(receipt: &Receipt) -> (Option<ExecutionStatus>, Option<String>) {
    if receipt.is_reverted() {
        (Some(ExecutionStatus::Reverted), receipt.revert_reason().map(|r| r.to_string()))
    } else {
        (Some(ExecutionStatus::Succeeded), None)
    }
}
