use katana_genesis::constant::DEFAULT_STRK_FEE_TOKEN_ADDRESS;
use katana_primitives::{felt, Felt};
use katana_rpc_api::txpool::TxPoolApiClient;
use katana_utils::TestNode;
use starknet::accounts::Account;
use starknet::core::types::Call;
use starknet::macros::selector;

/// Helper: creates a test node with no_mining so transactions stay in the pool.
async fn setup_no_mining_node() -> TestNode {
    let mut config = katana_utils::node::test_config();
    config.sequencing.no_mining = true;
    TestNode::new_with_config(config).await
}

/// Helper: submits a simple ERC-20 transfer invoke transaction.
/// Returns the transaction hash.
async fn send_transfer(
    account: &starknet::accounts::SingleOwnerAccount<
        starknet::providers::JsonRpcClient<starknet::providers::jsonrpc::HttpTransport>,
        starknet::signers::LocalWallet,
    >,
) -> Felt {
    let to = DEFAULT_STRK_FEE_TOKEN_ADDRESS.into();
    let selector = selector!("transfer");
    let calldata = vec![felt!("0x1"), felt!("0x1"), Felt::ZERO];

    let res = account
        .execute_v3(vec![Call { to, selector, calldata }])
        .l2_gas(100_000_000_000)
        .send()
        .await
        .unwrap();

    res.transaction_hash
}

#[tokio::test]
async fn txpool_status_empty_pool() {
    let node = setup_no_mining_node().await;
    let client = node.rpc_http_client();

    let status = client.txpool_status().await.unwrap();
    assert_eq!(status.pending, 0);
    assert_eq!(status.queued, 0);
}

#[tokio::test]
async fn txpool_status_after_submit() {
    let node = setup_no_mining_node().await;
    let client = node.rpc_http_client();
    let account = node.account();

    send_transfer(&account).await;

    let status = client.txpool_status().await.unwrap();
    assert_eq!(status.pending, 1);
    assert_eq!(status.queued, 0);
}

#[tokio::test]
async fn txpool_content_populated() {
    let node = setup_no_mining_node().await;
    let client = node.rpc_http_client();
    let account = node.account();

    let tx_hash = send_transfer(&account).await;

    let content = client.txpool_content().await.unwrap();

    // Should have exactly one sender in pending
    assert_eq!(content.pending.len(), 1);
    assert!(content.queued.is_empty());

    let sender_addr = account.address().into();
    let sender_txs = content.pending.get(&sender_addr).expect("sender should be present");
    assert_eq!(sender_txs.len(), 1);

    // Verify the transaction fields
    let tx_entry = sender_txs.values().next().unwrap();
    assert_eq!(tx_entry.hash, tx_hash);
    assert_eq!(tx_entry.sender, sender_addr);
}

#[tokio::test]
async fn txpool_content_from_filters_by_address() {
    let node = setup_no_mining_node().await;
    let client = node.rpc_http_client();
    let account = node.account();

    send_transfer(&account).await;

    let sender_addr = account.address().into();

    // Filter by the actual sender — should find the transaction
    let content = client.txpool_content_from(sender_addr).await.unwrap();
    assert_eq!(content.pending.len(), 1);
    assert!(content.pending.contains_key(&sender_addr));

    // Filter by an unrelated address — should be empty
    let other_addr = felt!("0xdead").into();
    let content = client.txpool_content_from(other_addr).await.unwrap();
    assert!(content.pending.is_empty());
}

#[tokio::test]
async fn txpool_inspect_format() {
    let node = setup_no_mining_node().await;
    let client = node.rpc_http_client();
    let account = node.account();

    send_transfer(&account).await;

    let inspect = client.txpool_inspect().await.unwrap();

    assert_eq!(inspect.pending.len(), 1);
    assert!(inspect.queued.is_empty());

    let sender_addr = account.address().into();
    let summaries = inspect.pending.get(&sender_addr).expect("sender should be present");
    let summary = summaries.values().next().unwrap();

    // The summary should contain key fields
    assert!(summary.contains("hash="), "summary should contain hash: {summary}");
    assert!(summary.contains("nonce="), "summary should contain nonce: {summary}");
    assert!(summary.contains("max_fee="), "summary should contain max_fee: {summary}");
    assert!(summary.contains("tip="), "summary should contain tip: {summary}");
}

#[tokio::test]
async fn txpool_cleared_after_block_produced() {
    // In instant mining mode, transactions are removed from the pool after the
    // block is produced via the BlockProductionTask polling loop.
    let node = TestNode::new().await;
    let client = node.rpc_http_client();
    let account = node.account();
    let provider = node.starknet_rpc_client();

    let tx_hash = send_transfer(&account).await;

    // Wait for the transaction to be included in a block
    katana_utils::TxWaiter::new(tx_hash, &provider).await.unwrap();

    // After the block is produced the pool should be drained.
    // Poll briefly in case removal is slightly async.
    let mut attempts = 0;
    loop {
        let status = client.txpool_status().await.unwrap();
        if status.pending == 0 {
            break;
        }
        attempts += 1;
        assert!(attempts < 50, "pool not drained after mining (pending={})", status.pending);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    let content = client.txpool_content().await.unwrap();
    assert!(content.pending.is_empty());
}

#[tokio::test]
async fn txpool_multiple_transactions() {
    let node = setup_no_mining_node().await;
    let client = node.rpc_http_client();
    let account = node.account();

    // Submit 3 transactions
    for _ in 0..3 {
        send_transfer(&account).await;
    }

    let status = client.txpool_status().await.unwrap();
    assert_eq!(status.pending, 3);

    let content = client.txpool_content().await.unwrap();
    let sender_addr = account.address().into();
    let sender_txs = content.pending.get(&sender_addr).expect("sender should be present");
    assert_eq!(sender_txs.len(), 3);
}
