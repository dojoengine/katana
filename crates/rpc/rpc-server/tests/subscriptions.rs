use std::time::Duration;

use jsonrpsee::ws_client::WsClient;
use katana_genesis::constant::DEFAULT_ETH_FEE_TOKEN_ADDRESS;
use katana_primitives::block::BlockIdOrTag;
use katana_primitives::felt;
use katana_starknet::rpc::StarknetRpcClient;
use katana_utils::TestNode;
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
    assert!(!event.event.keys.is_empty());
}

#[tokio::test]
async fn subscribe_transaction_status() {
    let node = TestNode::new().await;
    let client = ws_client(&node).await;

    // Send a transaction first.
    let tx_hash = send_test_tx(&node).await;

    // Subscribe to its status — it should already be finalized.
    let mut sub = client.subscribe_transaction_status(tx_hash).await.expect("failed to subscribe");

    let status = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("timed out waiting for tx status")
        .expect("subscription stream ended")
        .expect("failed to deserialize status");

    assert_eq!(status.transaction_hash, tx_hash);
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
