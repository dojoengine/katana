//! Integration tests for the Starknet gRPC server.
//!
//! Requirements:
//! - `git`, `asdf`, `scarb`, and `sozo` must be installed and properly configured
//! - Run with `cargo nextest run -p katana-grpc --features grpc`

#![cfg(feature = "grpc")]

use katana_grpc::proto::{
    BlockHashAndNumberRequest, BlockNumberRequest, BlockTag, ChainIdRequest, EventFilter,
    GetBlockRequest, GetClassAtRequest, GetClassHashAtRequest, GetEventsRequest, GetNonceRequest,
    GetStorageAtRequest, GetTransactionByHashRequest, GetTransactionReceiptRequest,
    SpecVersionRequest, SyncingRequest,
};
use katana_grpc::GrpcClient;
use katana_primitives::Felt;
use katana_utils::node::TestNode;
use tonic::Request;

/// Helper function to convert a Felt to a proto Felt.
fn felt_to_proto(felt: Felt) -> katana_grpc::proto::Felt {
    katana_grpc::proto::Felt { value: felt.to_bytes_be().to_vec() }
}

/// Helper function to convert a proto Felt to a Felt.
fn proto_to_felt(proto: &katana_grpc::proto::Felt) -> Felt {
    Felt::from_bytes_be_slice(&proto.value)
}

async fn setup() -> (TestNode, GrpcClient) {
    let node = TestNode::new().await;
    node.migrate_spawn_and_move().await.expect("migration failed");

    let grpc_addr = node.grpc_addr().expect("grpc not enabled");
    let client = GrpcClient::connect(format!("http://{}", grpc_addr))
        .await
        .expect("failed to connect to gRPC server");

    (node, client)
}

#[tokio::test]
async fn test_chain_id() {
    let (node, mut client) = setup().await;

    let response =
        client.chain_id(Request::new(ChainIdRequest {})).await.expect("chain_id call failed");
    let chain_id = response.into_inner().chain_id;

    // The test config uses SEPOLIA chain ID
    let expected_chain_id = node.backend().chain_spec.id();
    assert_eq!(chain_id, format!("{:#x}", expected_chain_id.id()));
}

#[tokio::test]
async fn test_block_number() {
    let (_node, mut client) = setup().await;

    let response = client
        .block_number(Request::new(BlockNumberRequest {}))
        .await
        .expect("block_number call failed");

    let block_number = response.into_inner().block_number;
    // After migration, block number should be greater than 0
    assert!(block_number > 0, "Expected block number > 0 after migration");
}

#[tokio::test]
async fn test_block_hash_and_number() {
    let (_node, mut client) = setup().await;

    let response = client
        .block_hash_and_number(Request::new(BlockHashAndNumberRequest {}))
        .await
        .expect("block_hash_and_number call failed");

    let inner = response.into_inner();
    let block_number = inner.block_number;
    let block_hash = inner.block_hash.expect("block_hash should be present");

    assert!(block_number > 0, "Expected block number > 0 after migration");
    let hash_felt = proto_to_felt(&block_hash);
    assert_ne!(hash_felt, Felt::ZERO, "Block hash should not be zero");
}

#[tokio::test]
async fn test_get_block_with_txs() {
    let (_node, mut client) = setup().await;

    // Get the genesis block (block 0)
    let request = GetBlockRequest {
        block_id: Some(katana_grpc::proto::BlockId {
            identifier: Some(katana_grpc::proto::block_id::Identifier::Number(0)),
        }),
    };

    let response = client
        .get_block_with_txs(Request::new(request))
        .await
        .expect("get_block_with_txs call failed");

    let inner = response.into_inner();
    assert!(inner.result.is_some(), "Expected a block in the response");
}

#[tokio::test]
async fn test_get_block_with_tx_hashes() {
    let (_node, mut client) = setup().await;

    // Get the genesis block (block 0)
    let request = GetBlockRequest {
        block_id: Some(katana_grpc::proto::BlockId {
            identifier: Some(katana_grpc::proto::block_id::Identifier::Number(0)),
        }),
    };

    let response = client
        .get_block_with_tx_hashes(Request::new(request))
        .await
        .expect("get_block_with_tx_hashes call failed");

    let inner = response.into_inner();
    assert!(inner.result.is_some(), "Expected a block in the response");
}

#[tokio::test]
async fn test_get_block_with_txs_latest() {
    let (_node, mut client) = setup().await;

    // Get the latest block
    let request = GetBlockRequest {
        block_id: Some(katana_grpc::proto::BlockId {
            identifier: Some(katana_grpc::proto::block_id::Identifier::Tag(
                BlockTag::Latest as i32,
            )),
        }),
    };

    let response = client
        .get_block_with_txs(Request::new(request))
        .await
        .expect("get_block_with_txs call failed");

    let inner = response.into_inner();
    assert!(inner.result.is_some(), "Expected a block in the response");
}

#[tokio::test]
async fn test_get_class_at() {
    let (node, mut client) = setup().await;

    // Get a genesis account address
    let (address, _) =
        node.backend().chain_spec.genesis().accounts().next().expect("must have genesis account");

    let request = GetClassAtRequest {
        block_id: Some(katana_grpc::proto::BlockId {
            identifier: Some(katana_grpc::proto::block_id::Identifier::Tag(
                BlockTag::Latest as i32,
            )),
        }),
        contract_address: Some(felt_to_proto((*address).into())),
    };

    let response =
        client.get_class_at(Request::new(request)).await.expect("get_class_at call failed");

    let inner = response.into_inner();
    assert!(inner.result.is_some(), "Expected a contract class in the response");
}

#[tokio::test]
async fn test_get_class_hash_at() {
    let (node, mut client) = setup().await;

    // Get a genesis account address
    let (address, _) =
        node.backend().chain_spec.genesis().accounts().next().expect("must have genesis account");

    let request = GetClassHashAtRequest {
        block_id: Some(katana_grpc::proto::BlockId {
            identifier: Some(katana_grpc::proto::block_id::Identifier::Tag(
                BlockTag::Latest as i32,
            )),
        }),
        contract_address: Some(felt_to_proto((*address).into())),
    };

    let response = client
        .get_class_hash_at(Request::new(request))
        .await
        .expect("get_class_hash_at call failed");

    let inner = response.into_inner();
    let class_hash = inner.class_hash.expect("class_hash should be present");
    let hash_felt = proto_to_felt(&class_hash);
    assert_ne!(hash_felt, Felt::ZERO, "Class hash should not be zero");
}

#[tokio::test]
async fn test_get_storage_at() {
    let (node, mut client) = setup().await;

    // Get a genesis account address
    let (address, _) =
        node.backend().chain_spec.genesis().accounts().next().expect("must have genesis account");

    // Storage key 0 is a common key to test
    let storage_key = Felt::ZERO;

    let request = GetStorageAtRequest {
        block_id: Some(katana_grpc::proto::BlockId {
            identifier: Some(katana_grpc::proto::block_id::Identifier::Tag(
                BlockTag::Latest as i32,
            )),
        }),
        contract_address: Some(felt_to_proto((*address).into())),
        key: Some(felt_to_proto(storage_key)),
    };

    let response =
        client.get_storage_at(Request::new(request)).await.expect("get_storage_at call failed");

    let inner = response.into_inner();
    assert!(inner.value.is_some(), "Expected a storage value in the response");
}

#[tokio::test]
async fn test_get_nonce() {
    let (node, mut client) = setup().await;

    // Get a genesis account address
    let (address, _) =
        node.backend().chain_spec.genesis().accounts().next().expect("must have genesis account");

    let request = GetNonceRequest {
        block_id: Some(katana_grpc::proto::BlockId {
            identifier: Some(katana_grpc::proto::block_id::Identifier::Tag(
                BlockTag::Latest as i32,
            )),
        }),
        contract_address: Some(felt_to_proto((*address).into())),
    };

    let response = client.get_nonce(Request::new(request)).await.expect("get_nonce call failed");

    let inner = response.into_inner();
    let nonce = inner.nonce.expect("nonce should be present");
    let nonce_felt = proto_to_felt(&nonce);
    // After migration, the account should have used some nonce
    assert!(nonce_felt > Felt::ZERO, "Nonce should be greater than zero after migration");
}

#[tokio::test]
async fn test_spec_version() {
    let (_node, mut client) = setup().await;

    let response = client
        .spec_version(Request::new(SpecVersionRequest {}))
        .await
        .expect("spec_version call failed");

    let version = response.into_inner().version;
    assert!(!version.is_empty(), "Expected a non-empty spec version");
}

#[tokio::test]
async fn test_syncing() {
    let (_node, mut client) = setup().await;

    let response =
        client.syncing(Request::new(SyncingRequest {})).await.expect("syncing call failed");

    let inner = response.into_inner();
    assert!(inner.result.is_some(), "Expected a syncing result");
}

#[tokio::test]
async fn test_get_block_transaction_count() {
    let (_node, mut client) = setup().await;

    // Get transaction count for genesis block (block 0)
    let request = GetBlockRequest {
        block_id: Some(katana_grpc::proto::BlockId {
            identifier: Some(katana_grpc::proto::block_id::Identifier::Number(0)),
        }),
    };

    let response = client
        .get_block_transaction_count(Request::new(request))
        .await
        .expect("get_block_transaction_count call failed");

    let count = response.into_inner().count;
    // Genesis block has no transactions
    assert_eq!(count, 0, "Expected no transactions in genesis block");
}

#[tokio::test]
async fn test_get_state_update() {
    let (_node, mut client) = setup().await;

    // Get state update for genesis block (block 0)
    let request = GetBlockRequest {
        block_id: Some(katana_grpc::proto::BlockId {
            identifier: Some(katana_grpc::proto::block_id::Identifier::Number(0)),
        }),
    };

    let response =
        client.get_state_update(Request::new(request)).await.expect("get_state_update call failed");

    let inner = response.into_inner();
    assert!(inner.result.is_some(), "Expected a state update in the response");
}

#[tokio::test]
async fn test_get_events() {
    let (_node, mut client) = setup().await;

    // Query events from block 0 to latest with no filters
    let request = GetEventsRequest {
        filter: Some(EventFilter {
            from_block: Some(katana_grpc::proto::BlockId {
                identifier: Some(katana_grpc::proto::block_id::Identifier::Number(0)),
            }),
            to_block: Some(katana_grpc::proto::BlockId {
                identifier: Some(katana_grpc::proto::block_id::Identifier::Tag(
                    BlockTag::Latest as i32,
                )),
            }),
            address: None,
            keys: vec![],
        }),
        chunk_size: 100,
        continuation_token: String::new(),
    };

    let response = client.get_events(Request::new(request)).await.expect("get_events call failed");

    let inner = response.into_inner();
    // After migration, there should be events emitted
    assert!(!inner.events.is_empty(), "Expected events after migration");
}

#[tokio::test]
async fn test_get_transaction_by_hash() {
    let (_node, mut client) = setup().await;

    // Get block 1 to find a transaction hash
    let block_request = GetBlockRequest {
        block_id: Some(katana_grpc::proto::BlockId {
            identifier: Some(katana_grpc::proto::block_id::Identifier::Number(1)),
        }),
    };

    let block_response = client
        .get_block_with_tx_hashes(Request::new(block_request))
        .await
        .expect("get_block_with_tx_hashes call failed");

    let block_inner = block_response.into_inner();

    // Extract a transaction hash from the block
    let tx_hash = match block_inner.result {
        Some(katana_grpc::proto::get_block_with_tx_hashes_response::Result::Block(block)) => {
            block.transactions.first().cloned()
        }
        Some(katana_grpc::proto::get_block_with_tx_hashes_response::Result::PendingBlock(
            pending,
        )) => pending.transactions.first().cloned(),
        None => None,
    };

    let tx_hash = tx_hash.expect("Expected at least one transaction in the block");

    // Now fetch the transaction by hash
    let request = GetTransactionByHashRequest { transaction_hash: Some(tx_hash) };

    let response = client
        .get_transaction_by_hash(Request::new(request))
        .await
        .expect("get_transaction_by_hash call failed");

    let inner = response.into_inner();
    assert!(inner.transaction.is_some(), "Expected a transaction in the response");
}

#[tokio::test]
async fn test_get_transaction_receipt() {
    let (_node, mut client) = setup().await;

    // Get block 1 to find a transaction hash
    let block_request = GetBlockRequest {
        block_id: Some(katana_grpc::proto::BlockId {
            identifier: Some(katana_grpc::proto::block_id::Identifier::Number(1)),
        }),
    };

    let block_response = client
        .get_block_with_tx_hashes(Request::new(block_request))
        .await
        .expect("get_block_with_tx_hashes call failed");

    let block_inner = block_response.into_inner();

    // Extract a transaction hash from the block
    let tx_hash = match block_inner.result {
        Some(katana_grpc::proto::get_block_with_tx_hashes_response::Result::Block(block)) => {
            block.transactions.first().cloned()
        }
        Some(katana_grpc::proto::get_block_with_tx_hashes_response::Result::PendingBlock(
            pending,
        )) => pending.transactions.first().cloned(),
        None => None,
    };

    let tx_hash = tx_hash.expect("Expected at least one transaction in the block");

    // Now fetch the transaction receipt
    let request = GetTransactionReceiptRequest { transaction_hash: Some(tx_hash) };

    let response = client
        .get_transaction_receipt(Request::new(request))
        .await
        .expect("get_transaction_receipt call failed");

    let inner = response.into_inner();
    assert!(inner.receipt.is_some(), "Expected a receipt in the response");
}
