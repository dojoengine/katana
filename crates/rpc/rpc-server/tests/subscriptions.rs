use std::time::Duration;

use jsonrpsee::ws_client::WsClient;
use katana_genesis::constant::DEFAULT_ETH_FEE_TOKEN_ADDRESS;
use katana_primitives::block::BlockIdOrTag;
use katana_primitives::{felt, Felt};
use katana_rpc_api::error::starknet::StarknetApiError;
use katana_rpc_types::receipt::{ExecutionResult, ReceiptBlockInfo};
use katana_rpc_types::subscription::ExecutionStatus;
use katana_starknet::rpc::{StarknetRpcClient, StarknetRpcClientError};
use katana_utils::{TestNode, TxWaiter};
use starknet::accounts::Account;
use starknet::core::types::Call;
use starknet::macros::selector;
use url::Url;

/// Helper to create a WS-backed `StarknetRpcClient` connected to the test node.
async fn ws_client(node: &TestNode) -> StarknetRpcClient<WsClient> {
    let url = Url::parse(&format!("ws://{}", node.rpc_addr())).expect("valid url");
    StarknetRpcClient::new_ws(url).await.expect("failed to connect WS client")
}

/// Helper to send an invoke transaction and wait until it's mined.
async fn send_test_tx(node: &TestNode) -> katana_primitives::Felt {
    let account = node.account();

    // Simple transfer to self — always succeeds on a dev node.
    let call = Call {
        to: DEFAULT_ETH_FEE_TOKEN_ADDRESS.into(),
        selector: selector!("transfer"),
        calldata: vec![account.address(), felt!("0x1"), felt!("0x0")],
    };

    let result = account.execute_v3(vec![call]).send().await.expect("failed to send tx");
    result.transaction_hash
}

/// Poll `get_transaction_receipt` until the receipt is available. Returns the receipt.
async fn wait_for_receipt(
    rpc: &StarknetRpcClient,
    tx_hash: Felt,
) -> katana_rpc_types::receipt::TxReceiptWithBlockInfo {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        match rpc.get_transaction_receipt(tx_hash).await {
            Ok(r) => return r,
            Err(StarknetRpcClientError::Starknet(StarknetApiError::TxnHashNotFound)) => {
                if tokio::time::Instant::now() >= deadline {
                    panic!("tx receipt never appeared");
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(e) => panic!("unexpected error fetching receipt: {e:?}"),
        }
    }
}

/// Submit a transaction whose execution will revert (transferring more ETH than the account
/// owns). Explicit resource bounds are set so `estimate_fee` is skipped — otherwise simulation
/// would error before the tx ever hits the pool.
async fn send_reverting_tx(node: &TestNode) -> Felt {
    let account = node.account();

    // Call a non-existent selector on the fee-token contract — the entry-point dispatcher will
    // panic, causing the account's `__execute__` to revert.
    let call = Call {
        to: DEFAULT_ETH_FEE_TOKEN_ADDRESS.into(),
        selector: selector!("this_selector_does_not_exist"),
        calldata: vec![],
    };

    let result = account
        .execute_v3(vec![call])
        .l1_gas(0)
        .l1_gas_price(1)
        .l2_gas(50_000_000)
        .l2_gas_price(1)
        .l1_data_gas(1024)
        .l1_data_gas_price(1)
        .send()
        .await
        .expect("failed to send reverting tx");
    result.transaction_hash
}

#[tokio::test]
async fn subscribe_new_heads_receives_block_header() {
    let node = TestNode::new().await;
    let client = ws_client(&node).await;

    let mut sub = client.subscribe_new_heads(None).await.expect("failed to subscribe");

    // Trigger a block by sending a transaction.
    send_test_tx(&node).await;

    // We should receive at least one block header notification.
    let header = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("timed out waiting for new head")
        .expect("subscription stream ended")
        .expect("failed to deserialize block header");

    assert!(header.block_number > 0);
    assert_ne!(header.block_hash, felt!("0x0"));
    assert_ne!(header.parent_hash, felt!("0x0"));
    assert!(!header.starknet_version.is_empty());
}

#[tokio::test]
async fn subscribe_new_heads_with_backfill() {
    let node = TestNode::new().await;
    let client = ws_client(&node).await;

    // Mine a few blocks first.
    send_test_tx(&node).await;
    send_test_tx(&node).await;

    // Subscribe starting from block 0 to get backfilled headers.
    let mut sub = client
        .subscribe_new_heads(Some(BlockIdOrTag::Number(0)))
        .await
        .expect("failed to subscribe");

    // Should receive backfilled headers starting from block 0.
    let first = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("timed out")
        .expect("stream ended")
        .expect("deserialization failed");

    assert_eq!(first.block_number, 0);
}

#[tokio::test]
async fn subscribe_new_heads_too_many_blocks_back() {
    let node = TestNode::new().await;
    let client = ws_client(&node).await;

    // Try to subscribe from a block hash that doesn't exist — the server should reject the
    // subscription with `BLOCK_NOT_FOUND`. (The actual "too many blocks back" path requires
    // >1024 blocks of history, which is impractical to set up here.)
    let result = client.subscribe_new_heads(Some(BlockIdOrTag::Hash(felt!("0xdeadbeef")))).await;

    assert!(result.is_err(), "expected subscription to be rejected for non-existent block");
}

#[tokio::test]
async fn subscribe_events_receives_events() {
    let node = TestNode::new().await;
    let client = ws_client(&node).await;

    // Subscribe to all events (no filter).
    let mut sub = client.subscribe_events(None, None, None).await.expect("failed to subscribe");

    // Send a transaction — ERC20 transfer emits a Transfer event.
    send_test_tx(&node).await;

    let event = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("timed out waiting for event")
        .expect("subscription stream ended")
        .expect("failed to deserialize event");

    assert!(event.event.block_number.is_some());
    assert!(event.event.block_hash.is_some());
    assert!(!event.event.event.keys.is_empty());
}

/// Path: tx is already in storage at subscribe-time, receipt is successful.
/// Server emits a single update with `execution_status = SUCCEEDED` then closes the stream.
#[tokio::test]
async fn subscribe_transaction_status_already_finalized_succeeded() {
    let node = TestNode::new().await;
    let client = ws_client(&node).await;

    // Send a tx and wait for it to be finalized in storage before subscribing.
    let tx_hash = send_test_tx(&node).await;
    TxWaiter::new(tx_hash, &node.starknet_rpc_client())
        .with_timeout(Duration::from_secs(10))
        .with_interval(50)
        .await
        .expect("tx never finalized");

    let mut sub = client.subscribe_transaction_status(tx_hash).await.expect("failed to subscribe");

    let status = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("timed out waiting for tx status")
        .expect("subscription stream ended")
        .expect("failed to deserialize status")
        .status;

    assert_eq!(status.execution_status, Some(ExecutionStatus::Succeeded));
    assert_eq!(status.failure_reason, None);
}

/// Path: tx is already in storage at subscribe-time, receipt is reverted.
/// Server emits a single update with `execution_status = REVERTED` and a non-empty
/// `failure_reason`.
#[tokio::test]
async fn subscribe_transaction_status_already_finalized_reverted() {
    let node = TestNode::new().await;
    let client = ws_client(&node).await;

    let tx_hash = send_reverting_tx(&node).await;
    let receipt = wait_for_receipt(&node.starknet_rpc_client(), tx_hash).await;
    assert!(
        matches!(receipt.receipt.execution_result(), ExecutionResult::Reverted { .. }),
        "expected tx to revert, got {:?}",
        receipt.receipt.execution_result()
    );

    let mut sub = client.subscribe_transaction_status(tx_hash).await.expect("failed to subscribe");

    let status = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("timed out waiting for tx status")
        .expect("subscription stream ended")
        .expect("failed to deserialize status")
        .status;

    assert_eq!(status.execution_status, Some(ExecutionStatus::Reverted));
    let reason = status.failure_reason.expect("reverted tx should carry a failure reason");
    assert!(!reason.is_empty(), "failure reason should not be empty");
}

/// Path: tx is in the pool (not yet mined) at subscribe-time.
/// Server emits a first "in-pool" update (no execution_status), then a second update once the
/// tx is mined.
#[tokio::test]
async fn subscribe_transaction_status_in_pool_then_mined() {
    // Long enough block time that the tx is guaranteed to sit in the pool when we subscribe.
    let node = TestNode::new_with_block_time(3000).await;
    let client = ws_client(&node).await;

    let tx_hash = send_test_tx(&node).await;

    let mut sub = client.subscribe_transaction_status(tx_hash).await.expect("failed to subscribe");

    // First notification: the tx was in the pool — execution_status is absent.
    let pool_status = tokio::time::timeout(Duration::from_secs(2), sub.next())
        .await
        .expect("timed out waiting for pool status")
        .expect("subscription stream ended")
        .expect("failed to deserialize pool status")
        .status;

    assert_eq!(pool_status.execution_status, None);
    assert_eq!(pool_status.failure_reason, None);

    // Second notification: the tx has been mined — execution_status is populated.
    let mined_status = tokio::time::timeout(Duration::from_secs(10), sub.next())
        .await
        .expect("timed out waiting for mined status")
        .expect("subscription stream ended")
        .expect("failed to deserialize mined status")
        .status;

    assert_eq!(mined_status.execution_status, Some(ExecutionStatus::Succeeded));
}

#[tokio::test]
async fn subscribe_new_transactions() {
    let node = TestNode::new().await;
    let client = ws_client(&node).await;

    let mut sub = client.subscribe_new_transactions(None).await.expect("failed to subscribe");

    // Send a transaction.
    let tx_hash = send_test_tx(&node).await;

    let notification = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("timed out waiting for new transaction")
        .expect("subscription stream ended")
        .expect("failed to deserialize transaction");

    assert_eq!(notification.transaction.transaction_hash, tx_hash);
}

#[tokio::test]
async fn subscribe_new_transaction_receipts() {
    let node = TestNode::new().await;
    let client = ws_client(&node).await;

    let mut sub =
        client.subscribe_new_transaction_receipts(None).await.expect("failed to subscribe");

    // Send a transaction — its receipt should land in the stream once the block is mined.
    let tx_hash = send_test_tx(&node).await;

    let receipt = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("timed out waiting for receipt")
        .expect("subscription stream ended")
        .expect("failed to deserialize receipt");

    assert_eq!(receipt.transaction_hash, tx_hash);
    let ReceiptBlockInfo::Block { block_number, .. } = receipt.block else {
        panic!("expected a confirmed block receipt, got {:?}", receipt.block);
    };
    assert!(block_number > 0);
}

#[tokio::test]
async fn subscribe_new_transaction_receipts_filters_by_sender() {
    let node = TestNode::new().await;
    let client = ws_client(&node).await;

    // Filter on an address that will never send a tx in this test.
    let other_sender = vec![felt!("0xdeadbeef").into()];
    let mut sub = client
        .subscribe_new_transaction_receipts(Some(other_sender))
        .await
        .expect("failed to subscribe");

    // Send a tx from the account — its receipt should be filtered out.
    send_test_tx(&node).await;

    // Give the server a chance to publish the (filtered-out) receipt; we expect a timeout.
    let result = tokio::time::timeout(Duration::from_secs(2), sub.next()).await;
    assert!(result.is_err(), "expected no receipt to be delivered, got {result:?}");
}

#[tokio::test]
async fn subscribe_new_heads_interval_mode() {
    // Use block-time mode to ensure blocks are produced at intervals.
    let node = TestNode::new_with_block_time(500).await;
    let client = ws_client(&node).await;

    let mut sub = client.subscribe_new_heads(None).await.expect("failed to subscribe");

    // Send a transaction to queue work — the block will be mined on the next interval tick.
    send_test_tx(&node).await;

    let header = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("timed out waiting for interval block")
        .expect("subscription stream ended")
        .expect("failed to deserialize block header");

    assert!(header.block_number > 0);
}
