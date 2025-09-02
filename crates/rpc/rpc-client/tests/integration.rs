use jsonrpsee::http_client::HttpClientBuilder;
use katana_primitives::block::{BlockIdOrTag, BlockTag};
use katana_primitives::contract::ContractAddress;
use katana_primitives::transaction::TxHash;
use katana_primitives::Felt;
use katana_rpc_api::error::starknet::StarknetApiError;
use katana_rpc_client::{Client, Error};

/// This test requires a running Katana instance on localhost:5050
/// Run with: cargo test --test integration -- --ignored
#[tokio::test]
#[ignore = "requires running katana instance"]
async fn test_client_basic_operations() {
    // Create the client
    let http_client = HttpClientBuilder::default()
        .build("http://localhost:5050")
        .expect("Failed to create HTTP client");

    let client = Client::new(http_client);

    // Test spec_version
    let version = client.spec_version().await.expect("Failed to get spec version");
    assert!(!version.is_empty());
    println!("Spec version: {}", version);

    // Test chain_id
    let chain_id = client.chain_id().await.expect("Failed to get chain ID");
    assert!(chain_id != Felt::ZERO);
    println!("Chain ID: {:#x}", chain_id);

    // Test block_number
    let block_number = client.block_number().await.expect("Failed to get block number");
    println!("Block number: {}", block_number.block_number);

    // Test block_hash_and_number
    let block_info =
        client.block_hash_and_number().await.expect("Failed to get block hash and number");
    println!("Block hash: {:#x}, number: {}", block_info.block_hash, block_info.block_number);

    // Test syncing status
    let sync_status = client.syncing().await.expect("Failed to get sync status");
    match sync_status {
        katana_rpc_types::SyncingStatus::NotSyncing => {
            println!("Node is not syncing");
        }
        katana_rpc_types::SyncingStatus::Syncing(status) => {
            println!("Node is syncing: {:?}", status);
        }
    }
}

#[tokio::test]
#[ignore = "requires running katana instance"]
async fn test_client_block_operations() {
    let http_client = HttpClientBuilder::default()
        .build("http://localhost:5050")
        .expect("Failed to create HTTP client");

    let client = Client::new(http_client);

    // Test get_block_with_tx_hashes
    let block = client
        .get_block_with_tx_hashes(BlockIdOrTag::Tag(BlockTag::Latest))
        .await
        .expect("Failed to get block with tx hashes");

    match block {
        katana_rpc_types::block::MaybePreConfirmedBlockWithTxHashes::Block(block) => {
            println!("Block hash: {:#x}", block.block_hash);
            println!("Transaction count: {}", block.transactions.len());
        }
        katana_rpc_types::block::MaybePreConfirmedBlockWithTxHashes::PreConfirmed(_) => {
            println!("Got pre-confirmed block");
        }
    }

    // Test get_block_with_txs
    let block = client
        .get_block_with_txs(BlockIdOrTag::Tag(BlockTag::Latest))
        .await
        .expect("Failed to get block with txs");

    match block {
        katana_rpc_types::block::MaybePreConfirmedBlockWithTxs::Block(block) => {
            println!("Block contains {} transactions", block.transactions.len());
        }
        katana_rpc_types::block::MaybePreConfirmedBlockWithTxs::PreConfirmed(_) => {
            println!("Got pre-confirmed block");
        }
    }

    // Test get_block_transaction_count
    let tx_count = client
        .get_block_transaction_count(BlockIdOrTag::Tag(BlockTag::Latest))
        .await
        .expect("Failed to get block transaction count");

    println!("Block transaction count: {}", tx_count);
}

#[tokio::test]
#[ignore = "requires running katana instance"]
async fn test_client_error_handling() {
    let http_client = HttpClientBuilder::default()
        .build("http://localhost:5050")
        .expect("Failed to create HTTP client");

    let client = Client::new(http_client);

    // Test BlockNotFound error
    let invalid_block = BlockIdOrTag::Number(999999999);
    match client.get_block_with_tx_hashes(invalid_block).await {
        Ok(_) => panic!("Expected BlockNotFound error"),
        Err(Error::Starknet(StarknetApiError::BlockNotFound)) => {
            println!("Got expected BlockNotFound error");
        }
        Err(e) => panic!("Unexpected error: {:?}", e),
    }

    // Test TxnHashNotFound error
    let invalid_tx_hash = TxHash::from_hex("0x123").expect("Failed to create tx hash");
    match client.get_transaction_by_hash(invalid_tx_hash).await {
        Ok(_) => panic!("Expected TxnHashNotFound error"),
        Err(Error::Starknet(StarknetApiError::TxnHashNotFound)) => {
            println!("Got expected TxnHashNotFound error");
        }
        Err(e) => panic!("Unexpected error: {:?}", e),
    }

    // Test ContractNotFound error
    let invalid_address =
        ContractAddress::new(Felt::from_hex("0x999").expect("Failed to create Felt"));
    match client.get_class_hash_at(BlockIdOrTag::Tag(BlockTag::Latest), invalid_address).await {
        Ok(_) => panic!("Expected ContractNotFound error"),
        Err(Error::Starknet(StarknetApiError::ContractNotFound)) => {
            println!("Got expected ContractNotFound error");
        }
        Err(e) => panic!("Unexpected error: {:?}", e),
    }
}

#[tokio::test]
#[ignore = "requires running katana instance"]
async fn test_client_state_operations() {
    let http_client = HttpClientBuilder::default()
        .build("http://localhost:5050")
        .expect("Failed to create HTTP client");

    let client = Client::new(http_client);

    // Test get_state_update
    let state_update = client
        .get_state_update(BlockIdOrTag::Tag(BlockTag::Latest))
        .await
        .expect("Failed to get state update");

    match state_update {
        katana_rpc_types::state_update::MaybePreConfirmedStateUpdate::Update(update) => {
            println!("State update: {:?}", update);
        }
        katana_rpc_types::state_update::MaybePreConfirmedStateUpdate::PreConfirmed(_) => {
            println!("Got pre-confirmed state update");
        }
    }

    // Test get_events with empty filter
    let filter = katana_rpc_types::event::EventFilterWithPage {
        event_filter: katana_rpc_types::event::EventFilter {
            from_block: Some(BlockIdOrTag::Number(0)),
            to_block: Some(BlockIdOrTag::Tag(BlockTag::Latest)),
            address: None,
            keys: None,
        },
        result_page_request: katana_rpc_types::event::ResultPageRequest {
            continuation_token: None,
            chunk_size: None,
        },
    };

    let events = client.get_events(filter).await.expect("Failed to get events");
    println!("Found {} events", events.events.len());
}

#[tokio::test]
#[ignore = "requires running katana instance"]
async fn test_client_transaction_trace() {
    let http_client = HttpClientBuilder::default()
        .build("http://localhost:5050")
        .expect("Failed to create HTTP client");

    let client = Client::new(http_client);

    // First, get a block to find a transaction
    let block = client
        .get_block_with_tx_hashes(BlockIdOrTag::Tag(BlockTag::Latest))
        .await
        .expect("Failed to get block");

    let tx_hash = match block {
        katana_rpc_types::block::MaybePreConfirmedBlockWithTxHashes::Block(block) => {
            if !block.transactions.is_empty() {
                Some(block.transactions[0])
            } else {
                None
            }
        }
        _ => None,
    };

    if let Some(tx_hash) = tx_hash {
        // Test trace_transaction
        let trace =
            client.trace_transaction(tx_hash).await.expect("Failed to get transaction trace");

        println!("Got transaction trace: {:?}", trace);
    } else {
        println!("No transactions found in latest block to trace");
    }

    // Test trace_block_transactions
    let traces = client
        .trace_block_transactions(BlockIdOrTag::Tag(BlockTag::Latest))
        .await
        .expect("Failed to get block traces");

    println!("Got {} transaction traces for the block", traces.len());
}
