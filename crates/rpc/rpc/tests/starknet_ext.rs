use assert_matches::assert_matches;
use katana_primitives::genesis::constant::DEFAULT_STRK_FEE_TOKEN_ADDRESS;
use katana_rpc_api::starknet_ext::StarknetApiExtClient;
use katana_rpc_types::list::{ContinuationToken, GetBlocksRequest, GetTransactionsRequest};
use katana_utils::TestNode;
use starknet::accounts::ConnectedAccount;
use starknet::core::types::{Felt, ResultPageRequest};

mod common;

use common::{Erc20Contract, Uint256};

#[tokio::test]
async fn get_blocks_basic_range() {
    let sequencer = TestNode::new().await;

    let client = sequencer.rpc_http_client();
    let account = sequencer.account();
    let erc20 = Erc20Contract::new(DEFAULT_STRK_FEE_TOKEN_ADDRESS.into(), &account);

    // function call params
    let recipient = Felt::ONE;
    let amount = Uint256 { low: Felt::ONE, high: Felt::ZERO };

    // Generate some blocks by sending transactions. Each transaction will generate a block. So we
    // expect to have 3 blocks - block 0 to 2.
    for _ in 0..3 {
        let result = erc20.transfer(&recipient, &amount).send().await.unwrap();
        katana_utils::TxWaiter::new(result.transaction_hash, &account.provider()).await.unwrap();
    }

    // Test getting blocks in range
    let request = GetBlocksRequest {
        from: 0,
        to: Some(2),
        result_page_request: ResultPageRequest { continuation_token: None, chunk_size: 10 },
    };

    let response = client.get_blocks(request).await.unwrap();

    // Should return blocks 0, 1, 2 (3 blocks total)
    assert_eq!(response.blocks.len(), 3);
    assert_eq!(response.blocks[0].0.block_number, 0);
    assert_eq!(response.blocks[1].0.block_number, 1);
    assert_eq!(response.blocks[2].0.block_number, 2);
    assert!(response.continuation_token.is_none());
}

#[tokio::test]
async fn get_blocks_with_chunk_size() {
    let sequencer = TestNode::new().await;

    let client = sequencer.rpc_http_client();
    let account = sequencer.account();
    let erc20 = Erc20Contract::new(DEFAULT_STRK_FEE_TOKEN_ADDRESS.into(), &account);

    // function call params
    let recipient = Felt::ONE;
    let amount = Uint256 { low: Felt::ONE, high: Felt::ZERO };

    // Generate some blocks by sending transactions. Each transaction will generate a block. So we
    // expect to have 5 blocks - block 0 to 4.
    for _ in 0..5 {
        let result = erc20.transfer(&recipient, &amount).send().await.unwrap();
        katana_utils::TxWaiter::new(result.transaction_hash, &account.provider()).await.unwrap();
    }

    // Request with chunk size limit
    let request = GetBlocksRequest {
        from: 0,
        to: Some(5),
        result_page_request: ResultPageRequest { continuation_token: None, chunk_size: 3 },
    };

    let response = client.get_blocks(request).await.unwrap();

    // Should return only first 3 blocks due to chunk size limit
    assert_eq!(response.blocks.len(), 3);
    assert_eq!(response.blocks[0].0.block_number, 0);
    assert_eq!(response.blocks[1].0.block_number, 1);
    assert_eq!(response.blocks[2].0.block_number, 2);

    // Should have continuation token since more blocks are available - starting from block 3
    // because we've only gotten until block 2.
    let expected_token = ContinuationToken { item_n: 3 };

    // Should have continuation token since more blocks are available
    assert_matches!(response.continuation_token, Some(token) => {
        let actual_token = ContinuationToken::parse(&token).unwrap();
        assert_eq!(actual_token, expected_token);
    });
}

#[tokio::test]
async fn get_blocks_pagination() {
    let sequencer = TestNode::new().await;

    let client = sequencer.rpc_http_client();
    let account = sequencer.account();
    let erc20 = Erc20Contract::new(DEFAULT_STRK_FEE_TOKEN_ADDRESS.into(), &account);

    // function call params
    let recipient = Felt::ONE;
    let amount = Uint256 { low: Felt::ONE, high: Felt::ZERO };

    // Generate blocks
    for _ in 0..5 {
        let result = erc20.transfer(&recipient, &amount).send().await.unwrap();
        katana_utils::TxWaiter::new(result.transaction_hash, &account.provider()).await.unwrap();
    }

    // First page
    let request = GetBlocksRequest {
        from: 0,
        to: Some(5),
        result_page_request: ResultPageRequest { continuation_token: None, chunk_size: 2 },
    };

    let first_response = client.get_blocks(request).await.unwrap();
    assert_eq!(first_response.blocks.len(), 2);
    assert_eq!(first_response.blocks[0].0.block_number, 0);
    assert_eq!(first_response.blocks[1].0.block_number, 1);
    assert!(first_response.continuation_token.is_some());

    // Second page using continuation token
    let request = GetBlocksRequest {
        from: 0,
        to: Some(5),
        result_page_request: ResultPageRequest {
            continuation_token: first_response.continuation_token.clone(),
            chunk_size: 2,
        },
    };

    let second_response = client.get_blocks(request).await.unwrap();
    assert_eq!(second_response.blocks.len(), 2);
    assert_eq!(second_response.blocks[0].0.block_number, 2);
    assert_eq!(second_response.blocks[1].0.block_number, 3);
    assert!(second_response.continuation_token.is_some());
}

#[tokio::test]
async fn get_blocks_no_to_parameter() {
    let sequencer = TestNode::new().await;

    let client = sequencer.rpc_http_client();
    let account = sequencer.account();
    let erc20 = Erc20Contract::new(DEFAULT_STRK_FEE_TOKEN_ADDRESS.into(), &account);

    // function call params
    let recipient = Felt::ONE;
    let amount = Uint256 { low: Felt::ONE, high: Felt::ZERO };

    // Generate some blocks
    for _ in 0..3 {
        let result = erc20.transfer(&recipient, &amount).send().await.unwrap();
        katana_utils::TxWaiter::new(result.transaction_hash, &account.provider()).await.unwrap();
    }

    // Test without 'to' parameter (should get from start to latest)
    let request = GetBlocksRequest {
        from: 1,
        to: None,
        result_page_request: ResultPageRequest { continuation_token: None, chunk_size: 10 },
    };

    let response = client.get_blocks(request).await.unwrap();

    // Should return blocks from 1 to latest
    assert!(response.blocks.len() >= 3); // At least blocks 1, 2, 3
    assert_eq!(response.blocks[0].0.block_number, 1);
}

#[tokio::test]
async fn get_transactions_basic_range() {
    let sequencer = TestNode::new().await;

    let client = sequencer.rpc_http_client();
    let account = sequencer.account();
    let erc20 = Erc20Contract::new(DEFAULT_STRK_FEE_TOKEN_ADDRESS.into(), &account);

    // function call params
    let recipient = Felt::ONE;
    let amount = Uint256 { low: Felt::ONE, high: Felt::ZERO };

    // Generate some transactions
    let mut tx_hashes = Vec::new();
    for _ in 0..3 {
        let result = erc20.transfer(&recipient, &amount).send().await.unwrap();
        let tx_hash = result.transaction_hash;
        tx_hashes.push(tx_hash);

        katana_utils::TxWaiter::new(tx_hash, &account.provider()).await.unwrap();
    }

    // Test getting transactions in range
    let request = GetTransactionsRequest {
        from: 0,
        to: Some(2),
        result_page_request: ResultPageRequest { continuation_token: None, chunk_size: 10 },
    };

    let response = client.get_transactions(request).await.unwrap();

    // Should return transactions 0, 1, 2
    assert_eq!(response.transactions.len(), 3);
    assert!(response.continuation_token.is_none());
}

#[tokio::test]
async fn get_transactions_with_chunk_size() {
    let sequencer = TestNode::new().await;

    let client = sequencer.rpc_http_client();
    let account = sequencer.account();
    let erc20 = Erc20Contract::new(DEFAULT_STRK_FEE_TOKEN_ADDRESS.into(), &account);

    // function call params
    let recipient = Felt::ONE;
    let amount = Uint256 { low: Felt::ONE, high: Felt::ZERO };

    // Generate some transactions
    let mut tx_hashes = Vec::new();
    for _ in 0..5 {
        let result = erc20.transfer(&recipient, &amount).send().await.unwrap();
        let tx_hash = result.transaction_hash;
        tx_hashes.push(tx_hash);

        katana_utils::TxWaiter::new(tx_hash, &account.provider()).await.unwrap();
    }

    // Request with chunk size limit
    let request = GetTransactionsRequest {
        from: 0,
        to: Some(5),
        result_page_request: ResultPageRequest { continuation_token: None, chunk_size: 3 },
    };

    let response = client.get_transactions(request).await.unwrap();
    let txs = response.transactions;

    // Should return only first 3 transactions due to chunk size limit
    assert_eq!(txs.len(), 3);
    // Should have continuation token since more transactions are available
    assert!(response.continuation_token.is_some());

    for (expected_hash, actual_tx) in tx_hashes.iter().zip(txs.iter()) {
        assert_eq!(expected_hash, actual_tx.0.transaction_hash());
    }
}

#[tokio::test]
async fn get_transactions_pagination() {
    let sequencer = TestNode::new().await;

    let client = sequencer.rpc_http_client();
    let account = sequencer.account();
    let erc20 = Erc20Contract::new(DEFAULT_STRK_FEE_TOKEN_ADDRESS.into(), &account);

    // function call params
    let recipient = Felt::ONE;
    let amount = Uint256 { low: Felt::ONE, high: Felt::ZERO };

    // Generate transactions
    for _ in 0..5 {
        let result = erc20.transfer(&recipient, &amount).send().await.unwrap();
        katana_utils::TxWaiter::new(result.transaction_hash, &account.provider()).await.unwrap();
    }

    // First page
    let request = GetTransactionsRequest {
        from: 0,
        to: Some(5),
        result_page_request: ResultPageRequest { continuation_token: None, chunk_size: 2 },
    };

    let first_response = client.get_transactions(request).await.unwrap();
    assert_eq!(first_response.transactions.len(), 2);
    assert!(first_response.continuation_token.is_some());

    // Second page using continuation token
    let request = GetTransactionsRequest {
        from: 0,
        to: Some(5),
        result_page_request: ResultPageRequest {
            continuation_token: first_response.continuation_token.clone(),
            chunk_size: 2,
        },
    };

    let second_response = client.get_transactions(request).await.unwrap();
    assert_eq!(second_response.transactions.len(), 2);
    assert!(second_response.continuation_token.is_some());
}

#[tokio::test]
async fn get_transactions_no_to_parameter() {
    let sequencer = TestNode::new().await;

    let client = sequencer.rpc_http_client();
    let account = sequencer.account();
    let erc20 = Erc20Contract::new(DEFAULT_STRK_FEE_TOKEN_ADDRESS.into(), &account);

    // function call params
    let recipient = Felt::ONE;
    let amount = Uint256 { low: Felt::ONE, high: Felt::ZERO };

    // Generate some transactions
    for _ in 0..3 {
        let result = erc20.transfer(&recipient, &amount).send().await.unwrap();
        katana_utils::TxWaiter::new(result.transaction_hash, &account.provider()).await.unwrap();
    }

    // Test without 'to' parameter (should get from start to latest)
    let request = GetTransactionsRequest {
        from: 1,
        to: None,
        result_page_request: ResultPageRequest { continuation_token: None, chunk_size: 10 },
    };

    let response = client.get_transactions(request).await.unwrap();

    // Should return transactions from 1 to latest
    assert!(response.transactions.len() >= 3);
}

#[tokio::test]
async fn transaction_number() {
    let sequencer = TestNode::new().await;

    let client = sequencer.rpc_http_client();
    let account = sequencer.account();
    let erc20 = Erc20Contract::new(DEFAULT_STRK_FEE_TOKEN_ADDRESS.into(), &account);

    // function call params
    let recipient = Felt::ONE;
    let amount = Uint256 { low: Felt::ONE, high: Felt::ZERO };

    // Initially should be 0 transactions
    let initial_count = client.transaction_number().await.unwrap();
    assert_eq!(initial_count, 0);

    // Generate some transactions
    for _ in 0..3 {
        let result = erc20.transfer(&recipient, &amount).send().await.unwrap();
        katana_utils::TxWaiter::new(result.transaction_hash, &account.provider()).await.unwrap();
    }

    // Should now have 3 more transactions
    let final_count = client.transaction_number().await.unwrap();
    assert_eq!(final_count, initial_count + 3);
}

#[tokio::test]
async fn empty_range_requests() {
    let sequencer = TestNode::new().await;
    let client = sequencer.rpc_http_client();

    // Test empty blocks range
    let blocks_request = GetBlocksRequest {
        from: 100, // Non-existent block
        to: Some(200),
        result_page_request: ResultPageRequest { continuation_token: None, chunk_size: 10 },
    };

    let blocks_response = client.get_blocks(blocks_request).await.unwrap();
    assert_eq!(blocks_response.blocks.len(), 0);
    assert!(blocks_response.continuation_token.is_none());

    // Test empty transactions range
    let txs_request = GetTransactionsRequest {
        from: 100, // Non-existent transaction
        to: Some(200),
        result_page_request: ResultPageRequest { continuation_token: None, chunk_size: 10 },
    };

    let txs_response = client.get_transactions(txs_request).await.unwrap();
    assert_eq!(txs_response.transactions.len(), 0);
    assert!(txs_response.continuation_token.is_none());
}
