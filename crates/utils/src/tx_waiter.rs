use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use anyhow::Result;
use futures::FutureExt;
pub use katana_primitives::block::FinalityStatus;
use katana_primitives::Felt;
use katana_rpc_api::error::starknet::StarknetApiError;
use katana_rpc_client::starknet::{Client as StarknetClient, Error as StarknetClientError};
use katana_rpc_types::receipt::{
    ExecutionResult, ReceiptBlockInfo, RpcTxReceipt, TxReceiptWithBlockInfo,
};
use katana_rpc_types::TxStatus;
use tokio::time::{Instant, Interval};

type GetTxStatusResult = Result<TxStatus, StarknetClientError>;
type GetTxReceiptResult = Result<TxReceiptWithBlockInfo, StarknetClientError>;

type GetTxStatusFuture<'a> = Pin<Box<dyn Future<Output = GetTxStatusResult> + Send + 'a>>;
type GetTxReceiptFuture<'a> = Pin<Box<dyn Future<Output = GetTxReceiptResult> + Send + 'a>>;

#[derive(Debug, thiserror::Error)]
pub enum TxWaitingError {
    #[error("request timed out")]
    Timeout,

    #[error("transaction reverted with reason: {0}")]
    TransactionReverted(String),

    #[error(transparent)]
    Client(StarknetClientError),
}

/// Utility for waiting on a transaction.
///
/// The waiter will poll for the transaction receipt every `interval` miliseconds until it achieves
/// the desired status or until `timeout` is reached.
///
/// The waiter can be configured to wait for a specific finality status (e.g, `ACCEPTED_ON_L2`), by
/// default, it only waits until the transaction is included in the _pending_ block. It can also be
/// set to check if the transaction is executed successfully or not (reverted).
///
/// # Examples
///
/// ```ignore
/// use url::Url;
/// use starknet::providers::jsonrpc::HttpTransport;
/// use starknet::providers::JsonRpcClient;
/// use starknet::core::types::FinalityStatus;
///
/// let provider = JsonRpcClient::new(HttpTransport::new(Url::parse("http://localhost:5000").unwrap()));
///
/// let tx_hash = Felt::from(0xbadbeefu64);
/// let receipt = TxWaiter::new(tx_hash, &provider).with_tx_status(FinalityStatus::AcceptedOnL2).await.unwrap();
/// ```
#[must_use = "TxWaiter does nothing unless polled"]
pub struct TxWaiter<'a> {
    /// The hash of the transaction to wait for.
    tx_hash: Felt,
    /// The transaction finality status to wait for.
    ///
    /// If not set, then it will wait until it is `ACCEPTED_ON_L2` whether it is reverted or not.
    tx_finality_status: Option<FinalityStatus>,
    /// A flag to indicate that the waited transaction must either be successfully executed or not.
    ///
    /// If it's set to `true`, then the transaction execution result must be `SUCCEEDED` otherwise
    /// an error will be returned. However, if set to `false`, then the execution status will not
    /// be considered when waiting for the transaction, meaning `REVERTED` transaction will not
    /// return an error.
    must_succeed: bool,
    /// Poll the transaction every `interval` milliseconds. Milliseconds are used so that
    /// we can be more precise with the polling interval. Defaults to 2.5 seconds.
    interval: Interval,
    /// The maximum amount of time to wait for the transaction to achieve the desired status. An
    /// error will be returned if it is unable to finish within the `timeout` duration. Defaults to
    /// 300 seconds.
    timeout: Duration,
    /// The provider to use for polling the transaction.
    rpc_client: &'a StarknetClient,

    /// The future that gets the transaction status.
    tx_status_request_fut: Option<GetTxStatusFuture<'a>>,
    /// The future that gets the transaction receipt.
    tx_receipt_request_fut: Option<GetTxReceiptFuture<'a>>,
    /// The time when the transaction waiter was first polled.
    started_at: Option<Instant>,
}

impl<'a> TxWaiter<'a> {
    /// The default timeout for a transaction to be accepted or reverted on L2.
    /// The inclusion (which can be accepted or reverted) is ~5seconds in ideal cases.
    /// We keep some margin for times that could be affected by network congestion or
    /// block STM worst cases.
    const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
    /// Interval for use with 3rd party provider without burning the API rate limit.
    const DEFAULT_INTERVAL: Duration = Duration::from_millis(2500);

    pub fn new(tx: Felt, rpc_client: &'a StarknetClient) -> Self {
        Self {
            rpc_client,
            tx_hash: tx,
            started_at: None,
            must_succeed: true,
            tx_finality_status: None,
            tx_status_request_fut: None,
            tx_receipt_request_fut: None,
            timeout: Self::DEFAULT_TIMEOUT,
            interval: tokio::time::interval_at(
                Instant::now() + Self::DEFAULT_INTERVAL,
                Self::DEFAULT_INTERVAL,
            ),
        }
    }

    pub fn with_interval(self, milisecond: u64) -> Self {
        let interval = Duration::from_millis(milisecond);
        Self { interval: tokio::time::interval_at(Instant::now() + interval, interval), ..self }
    }

    pub fn with_tx_status(self, status: FinalityStatus) -> Self {
        Self { tx_finality_status: Some(status), ..self }
    }

    pub fn with_timeout(self, timeout: Duration) -> Self {
        Self { timeout, ..self }
    }

    // Helper function to evaluate if the transaction receipt should be accepted yet or not, based
    // on the waiter's parameters. Used in the `Future` impl.
    fn evaluate_receipt_from_params(
        receipt: TxReceiptWithBlockInfo,
        expected_finality_status: Option<FinalityStatus>,
        must_succeed: bool,
    ) -> Option<Result<TxReceiptWithBlockInfo, TxWaitingError>> {
        match &receipt.block {
            ReceiptBlockInfo::PreConfirmed { .. } => {
                // pending receipt doesn't include finality status, so we cant check it.
                if expected_finality_status.is_some() {
                    return None;
                }

                if !must_succeed {
                    return Some(Ok(receipt));
                }

                match execution_status_from_receipt(&receipt.receipt) {
                    ExecutionResult::Succeeded => Some(Ok(receipt)),
                    ExecutionResult::Reverted { reason } => {
                        Some(Err(TxWaitingError::TransactionReverted(reason.clone())))
                    }
                }
            }

            ReceiptBlockInfo::Block { .. } => {
                if let Some(expected_status) = expected_finality_status {
                    match finality_status_from_receipt(&receipt.receipt) {
                        FinalityStatus::AcceptedOnL2
                            if expected_status == FinalityStatus::AcceptedOnL1 =>
                        {
                            None
                        }

                        _ => {
                            if !must_succeed {
                                return Some(Ok(receipt));
                            }

                            match execution_status_from_receipt(&receipt.receipt) {
                                ExecutionResult::Succeeded => Some(Ok(receipt)),
                                ExecutionResult::Reverted { reason } => {
                                    Some(Err(TxWaitingError::TransactionReverted(reason.clone())))
                                }
                            }
                        }
                    }
                } else {
                    Some(Ok(receipt))
                }
            }
        }
    }
}

impl Future for TxWaiter<'_> {
    type Output = Result<TxReceiptWithBlockInfo, TxWaitingError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        if this.started_at.is_none() {
            this.started_at = Some(Instant::now());
        }

        loop {
            if let Some(started_at) = this.started_at {
                if started_at.elapsed() > this.timeout {
                    return Poll::Ready(Err(TxWaitingError::Timeout));
                }
            }

            if let Some(mut fut) = this.tx_status_request_fut.take() {
                match fut.poll_unpin(cx) {
                    Poll::Ready(res) => match res {
                        Ok(status) => match status {
                            TxStatus::PreConfirmed(_)
                            | TxStatus::AcceptedOnL2(_)
                            | TxStatus::AcceptedOnL1(_) => {
                                this.tx_receipt_request_fut = Some(Box::pin(
                                    this.rpc_client.get_transaction_receipt(this.tx_hash),
                                ));
                            }

                            TxStatus::Candidate | TxStatus::Received => {}
                        },

                        Err(StarknetClientError::Starknet(StarknetApiError::TxnHashNotFound)) => {}

                        Err(e) => {
                            return Poll::Ready(Err(TxWaitingError::Client(e)));
                        }
                    },

                    Poll::Pending => {
                        this.tx_status_request_fut = Some(fut);
                        return Poll::Pending;
                    }
                }
            }

            if let Some(mut fut) = this.tx_receipt_request_fut.take() {
                match fut.poll_unpin(cx) {
                    Poll::Pending => {
                        this.tx_receipt_request_fut = Some(fut);
                        return Poll::Pending;
                    }

                    Poll::Ready(res) => match res {
                        Err(StarknetClientError::Starknet(StarknetApiError::TxnHashNotFound)) => {}
                        Err(e) => {
                            return Poll::Ready(Err(TxWaitingError::Client(e)));
                        }

                        Ok(res) => {
                            if let Some(res) = Self::evaluate_receipt_from_params(
                                res,
                                this.tx_finality_status,
                                this.must_succeed,
                            ) {
                                return Poll::Ready(res);
                            }
                        }
                    },
                }
            }

            if this.interval.poll_tick(cx).is_ready() {
                this.tx_status_request_fut =
                    Some(Box::pin(this.rpc_client.get_transaction_status(this.tx_hash)));
            } else {
                break;
            }
        }

        Poll::Pending
    }
}

impl std::fmt::Debug for TxWaiter<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TxWaiter")
            .field("tx_hash", &self.tx_hash)
            .field("tx_finality_status", &self.tx_finality_status)
            .field("must_succeed", &self.must_succeed)
            .field("interval", &self.interval)
            .field("timeout", &self.timeout)
            .field("provider", &"..")
            .field("tx_status_request_fut", &"..")
            .field("tx_receipt_request_fut", &"..")
            .field("started_at", &self.started_at)
            .finish()
    }
}

fn execution_status_from_receipt(receipt: &RpcTxReceipt) -> &ExecutionResult {
    match receipt {
        RpcTxReceipt::Invoke(receipt) => &receipt.execution_result,
        RpcTxReceipt::Deploy(receipt) => &receipt.execution_result,
        RpcTxReceipt::Declare(receipt) => &receipt.execution_result,
        RpcTxReceipt::L1Handler(receipt) => &receipt.execution_result,
        RpcTxReceipt::DeployAccount(receipt) => &receipt.execution_result,
    }
}

fn finality_status_from_receipt(receipt: &RpcTxReceipt) -> FinalityStatus {
    match receipt {
        RpcTxReceipt::Invoke(receipt) => receipt.finality_status,
        RpcTxReceipt::Deploy(receipt) => receipt.finality_status,
        RpcTxReceipt::Declare(receipt) => receipt.finality_status,
        RpcTxReceipt::L1Handler(receipt) => receipt.finality_status,
        RpcTxReceipt::DeployAccount(receipt) => receipt.finality_status,
    }
}

#[cfg(test)]
mod tests {
    use assert_matches::assert_matches;
    use katana_primitives::block::FinalityStatus::{self, AcceptedOnL1, AcceptedOnL2};
    use katana_primitives::fee::PriceUnit;
    use katana_rpc_types::receipt::ExecutionResult::{Reverted, Succeeded};
    use katana_rpc_types::receipt::{
        ExecutionResult, ReceiptBlockInfo, RpcInvokeTxReceipt, RpcTxReceipt, TxReceiptWithBlockInfo,
    };
    use katana_rpc_types::{ExecutionResources, FeePayment};
    use starknet::macros::felt;

    use super::{Duration, TxWaiter};
    use crate::{TestNode, TxWaitingError};

    async fn create_test_sequencer() -> TestNode {
        TestNode::new().await
    }

    const EXECUTION_RESOURCES: ExecutionResources =
        ExecutionResources { l1_gas: 0, l2_gas: 0, l1_data_gas: 0 };

    fn mock_receipt(
        finality_status: FinalityStatus,
        execution_result: ExecutionResult,
    ) -> TxReceiptWithBlockInfo {
        let receipt = RpcTxReceipt::Invoke(RpcInvokeTxReceipt {
            finality_status,
            execution_result,
            events: Default::default(),
            actual_fee: FeePayment { amount: Default::default(), unit: PriceUnit::Wei },
            messages_sent: Default::default(),
            execution_resources: EXECUTION_RESOURCES,
        });

        TxReceiptWithBlockInfo {
            receipt,
            transaction_hash: Default::default(),
            block: ReceiptBlockInfo::Block {
                block_hash: Default::default(),
                block_number: Default::default(),
            },
        }
    }

    fn mock_preconf_receipt(execution_result: ExecutionResult) -> TxReceiptWithBlockInfo {
        let receipt = RpcTxReceipt::Invoke(RpcInvokeTxReceipt {
            execution_result,
            events: Default::default(),
            finality_status: FinalityStatus::AcceptedOnL2,
            actual_fee: FeePayment { amount: Default::default(), unit: PriceUnit::Wei },
            messages_sent: Default::default(),
            execution_resources: EXECUTION_RESOURCES,
        });

        TxReceiptWithBlockInfo {
            receipt,
            transaction_hash: Default::default(),
            block: ReceiptBlockInfo::PreConfirmed { block_number: 0 },
        }
    }

    #[tokio::test]
    async fn should_timeout_on_nonexistant_transaction() {
        let sequencer = create_test_sequencer().await;

        let client = sequencer.starknet_rpc_client();

        let hash = felt!("0x1234");
        let result =
            TxWaiter::new(hash, &client).with_timeout(Duration::from_secs(1)).await.unwrap_err();

        assert_matches!(result, TxWaitingError::Timeout);
    }

    macro_rules! eval_receipt {
        ($receipt:expr, $must_succeed:expr) => {
            TxWaiter::evaluate_receipt_from_params($receipt, None, $must_succeed)
        };

        ($receipt:expr, $expected_status:expr, $must_succeed:expr) => {
            TxWaiter::evaluate_receipt_from_params($receipt, Some($expected_status), $must_succeed)
        };
    }

    #[test]
    fn wait_for_no_finality_status() {
        let receipt = mock_receipt(AcceptedOnL2, Succeeded);
        assert!(eval_receipt!(receipt.clone(), false).unwrap().is_ok());
    }

    #[test]
    fn wait_for_finality_status_with_no_succeed() {
        let receipt = mock_receipt(AcceptedOnL2, Succeeded);
        assert!(eval_receipt!(receipt.clone(), AcceptedOnL2, false).unwrap().is_ok());

        let receipt = mock_receipt(AcceptedOnL2, Succeeded);
        assert!(eval_receipt!(receipt.clone(), AcceptedOnL1, true).is_none());

        let receipt = mock_receipt(AcceptedOnL1, Succeeded);
        assert!(eval_receipt!(receipt.clone(), AcceptedOnL2, false).unwrap().is_ok());

        let receipt = mock_receipt(AcceptedOnL1, Succeeded);
        assert!(eval_receipt!(receipt.clone(), AcceptedOnL1, false).unwrap().is_ok());
    }

    #[test]
    fn wait_for_finality_status_with_must_succeed() {
        let receipt = mock_receipt(AcceptedOnL2, Succeeded);
        assert!(eval_receipt!(receipt.clone(), AcceptedOnL2, true).unwrap().is_ok());

        let receipt = mock_receipt(AcceptedOnL1, Succeeded);
        assert!(eval_receipt!(receipt.clone(), AcceptedOnL2, true).unwrap().is_ok());

        let receipt = mock_receipt(AcceptedOnL1, Reverted { reason: Default::default() });
        let evaluation = eval_receipt!(receipt.clone(), AcceptedOnL1, true).unwrap();
        assert_matches!(evaluation, Err(TxWaitingError::TransactionReverted(_)));
    }

    #[test]
    fn wait_for_pending_tx() {
        let receipt = mock_preconf_receipt(Succeeded);
        assert!(eval_receipt!(receipt.clone(), AcceptedOnL2, true).is_none());

        let receipt = mock_preconf_receipt(Reverted { reason: Default::default() });
        assert!(eval_receipt!(receipt.clone(), false).unwrap().is_ok());

        let receipt = mock_preconf_receipt(Reverted { reason: Default::default() });
        let evaluation = eval_receipt!(receipt.clone(), true).unwrap();
        assert_matches!(evaluation, Err(TxWaitingError::TransactionReverted(_)));
    }
}
