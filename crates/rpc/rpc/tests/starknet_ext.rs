use assert_matches::assert_matches;
use katana_primitives::genesis::constant::DEFAULT_STRK_FEE_TOKEN_ADDRESS;
use katana_rpc_api::starknet_ext::StarknetApiExtClient;
use katana_rpc_types::list::{ContinuationToken, GetBlocksRequest, GetTransactionsRequest};
use katana_utils::node::Provider;
use katana_utils::TestNode;
use starknet::accounts::ConnectedAccount;
use starknet::core::types::{Felt, ResultPageRequest, TransactionReceipt};

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

    let latest_block = account.provider().block_number().await.unwrap();

    // Generate blocks
    //
    // we want to make sure the node only has 5 blocks for this test.
    for _ in 0..(5 - (latest_block + 1)) {
        let result = erc20.transfer(&recipient, &amount).send().await.unwrap();
        katana_utils::TxWaiter::new(result.transaction_hash, &account.provider()).await.unwrap();
    }

    // First page
    let request = GetBlocksRequest {
        from: 0,
        to: Some(4),
        result_page_request: ResultPageRequest { continuation_token: None, chunk_size: 2 },
    };

    let first_response = client.get_blocks(request).await.unwrap();
    assert_eq!(first_response.blocks.len(), 2);
    assert_eq!(first_response.blocks[0].0.block_number, 0);
    assert_eq!(first_response.blocks[1].0.block_number, 1);

    // Should have continuation token since we have 3 more block
    assert_matches!(&first_response.continuation_token, Some(token) => {
        use katana_rpc_types::list::ContinuationToken;
        let token = ContinuationToken::parse(&token).unwrap();
        assert_eq!(token.item_n, 2);
    });

    // Second page using continuation token
    let request = GetBlocksRequest {
        from: 0,
        to: Some(4),
        result_page_request: ResultPageRequest {
            continuation_token: first_response.continuation_token.clone(),
            chunk_size: 2,
        },
    };

    let second_response = client.get_blocks(request).await.unwrap();
    assert_eq!(second_response.blocks.len(), 2);
    assert_eq!(second_response.blocks[0].0.block_number, 2);
    assert_eq!(second_response.blocks[1].0.block_number, 3);

    // Should have continuation token since we have 1 more block
    assert_matches!(&second_response.continuation_token, Some(token) => {
        use katana_rpc_types::list::ContinuationToken;
        let token = ContinuationToken::parse(&token).unwrap();
        assert_eq!(token.item_n, 4);
    });

    // Third page using continuation token
    let request = GetBlocksRequest {
        from: 0,
        to: Some(4),
        result_page_request: ResultPageRequest {
            continuation_token: second_response.continuation_token.clone(),
            chunk_size: 2,
        },
    };

    let third_response = client.get_blocks(request).await.unwrap();
    assert_eq!(third_response.blocks.len(), 1);
    assert_eq!(third_response.blocks[0].0.block_number, 4);
    assert!(third_response.continuation_token.is_none());
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
    assert!(response.continuation_token.is_none());
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
        tx_hashes.push(result.transaction_hash);
        katana_utils::TxWaiter::new(result.transaction_hash, &account.provider()).await.unwrap();
    }

    // Request with chunk size limit
    let request = GetTransactionsRequest {
        from: 0,
        to: Some(4),
        result_page_request: ResultPageRequest { continuation_token: None, chunk_size: 3 },
    };

    let response = client.get_transactions(request).await.unwrap();

    // Should return only first 3 transactions (ie 0, 1, 2) due to chunk size limit
    assert_eq!(response.transactions.len(), 3);
    // Should have continuation token since more transactions are available
    assert_matches!(response.continuation_token, Some(token) => {
        use katana_rpc_types::list::ContinuationToken;
        let token = ContinuationToken::parse(&token).unwrap();
        assert_eq!(token.item_n, 3);
    });

    for (expected_hash, actual_tx) in tx_hashes.iter().take(3).zip(response.transactions.iter()) {
        assert_matches!(&actual_tx.0.receipt, TransactionReceipt::Invoke(receipt) => {
           assert_eq!(expected_hash, &receipt.transaction_hash);
        });
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
    let mut tx_hashes = Vec::new();
    for _ in 0..5 {
        let result = erc20.transfer(&recipient, &amount).send().await.unwrap();
        tx_hashes.push(result.transaction_hash);
        katana_utils::TxWaiter::new(result.transaction_hash, &account.provider()).await.unwrap();
    }

    // First page
    let request = GetTransactionsRequest {
        from: 0,
        to: Some(4),
        result_page_request: ResultPageRequest { continuation_token: None, chunk_size: 2 },
    };

    let first_response = client.get_transactions(request).await.unwrap();
    assert_eq!(first_response.transactions.len(), 2);
    // Should have continuation token since more transactions are available
    assert_matches!(&first_response.continuation_token, Some(token) => {
        use katana_rpc_types::list::ContinuationToken;
        let token = ContinuationToken::parse(token).unwrap();
        // We have only collected 2 transactions so far (ie tx 0, and tx 1), so the next
        // tx to fetch should be tx 2.
        assert_eq!(token.item_n, 2);
    });

    for (expected_hash, actual_tx) in
        tx_hashes.iter().take(2).zip(first_response.transactions.iter())
    {
        assert_matches!(&actual_tx.0.receipt, TransactionReceipt::Invoke(receipt) => {
           assert_eq!(expected_hash, &receipt.transaction_hash);
        });
    }

    // Second page using continuation token
    let request = GetTransactionsRequest {
        from: 0,
        to: Some(4),
        result_page_request: ResultPageRequest {
            continuation_token: first_response.continuation_token.clone(),
            chunk_size: 2,
        },
    };

    let second_response = client.get_transactions(request).await.unwrap();
    assert_eq!(second_response.transactions.len(), 2);
    // Should have continuation token since more transactions are available
    assert_matches!(&second_response.continuation_token, Some(token) => {
        use katana_rpc_types::list::ContinuationToken;
        let token = ContinuationToken::parse(token).unwrap();
        // We have only collected 4 transactions so far (ie tx 0, 1, 2, 3), so the next
        // tx to fetch should be tx 4.
        assert_eq!(token.item_n, 4);
    });

    for (expected_hash, actual_tx) in
        tx_hashes.iter().skip(2).zip(second_response.transactions.iter())
    {
        assert_matches!(&actual_tx.0.receipt, TransactionReceipt::Invoke(receipt) => {
           assert_eq!(expected_hash, &receipt.transaction_hash);
        });
    }

    // Third page using continuation token
    let request = GetTransactionsRequest {
        from: 0,
        to: Some(4),
        result_page_request: ResultPageRequest {
            continuation_token: second_response.continuation_token.clone(),
            chunk_size: 2,
        },
    };

    let third_response = client.get_transactions(request).await.unwrap();
    assert_eq!(third_response.transactions.len(), 1);
    assert!(third_response.continuation_token.is_none());

    for (expected_hash, actual_tx) in
        tx_hashes.iter().skip(4).zip(third_response.transactions.iter())
    {
        assert_matches!(&actual_tx.0.receipt, TransactionReceipt::Invoke(receipt) => {
           assert_eq!(expected_hash, &receipt.transaction_hash);
        });
    }
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
    let mut tx_hashes = Vec::new();
    for _ in 0..3 {
        let result = erc20.transfer(&recipient, &amount).send().await.unwrap();
        tx_hashes.push(result.transaction_hash);
        katana_utils::TxWaiter::new(result.transaction_hash, &account.provider()).await.unwrap();
    }

    // Test without 'to' parameter (should get from start to latest)
    let request = GetTransactionsRequest {
        from: 1,
        to: None,
        result_page_request: ResultPageRequest { continuation_token: None, chunk_size: 10 },
    };

    let response = client.get_transactions(request).await.unwrap();

    // Should return transactions from 1 to latest (3)
    assert_eq!(response.transactions.len(), 2);
    assert!(response.continuation_token.is_none());

    for (expected_hash, actual_tx) in tx_hashes.iter().skip(1).zip(response.transactions.iter()) {
        assert_matches!(&actual_tx.0.receipt, TransactionReceipt::Invoke(receipt) => {
           assert_eq!(expected_hash, &receipt.transaction_hash);
        });
    }
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
