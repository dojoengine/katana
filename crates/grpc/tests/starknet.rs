use katana_grpc::proto::{
    BlockHashAndNumberRequest, BlockNumberRequest, BlockTag, ChainIdRequest, GetBlockRequest,
    GetClassAtRequest, GetClassHashAtRequest, GetEventsRequest, GetNonceRequest,
    GetStorageAtRequest, GetTransactionByHashRequest, GetTransactionReceiptRequest,
    SpecVersionRequest, SyncingRequest,
};
use katana_grpc::GrpcClient;
use katana_primitives::Felt;
use katana_utils::node::TestNode;
use starknet::core::types::{BlockId, BlockTag as StarknetBlockTag, EventFilter};
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider, Url};
use tonic::Request;

async fn setup() -> (TestNode, GrpcClient, JsonRpcClient<HttpTransport>) {
    let node = TestNode::new_with_spawn_and_move_db().await;

    let grpc_addr = *node.grpc_addr().expect("grpc not enabled");
    let grpc = GrpcClient::connect(format!("http://{grpc_addr}"))
        .await
        .expect("failed to connect to gRPC server");

    let rpc_addr = *node.rpc_addr();
    let url = Url::parse(&format!("http://{rpc_addr}")).expect("failed to parse url");
    let rpc = JsonRpcClient::new(HttpTransport::new(url));

    (node, grpc, rpc)
}

fn genesis_address(node: &TestNode) -> Felt {
    let (address, _) =
        node.backend().chain_spec.genesis().accounts().next().expect("must have genesis account");
    (*address).into()
}

fn felt_to_proto(felt: Felt) -> katana_grpc::proto::Felt {
    katana_grpc::proto::Felt { value: felt.to_bytes_be().to_vec() }
}

fn proto_to_felt(proto: &katana_grpc::proto::Felt) -> Felt {
    Felt::from_bytes_be_slice(&proto.value)
}

fn grpc_block_id_number(n: u64) -> Option<katana_grpc::proto::BlockId> {
    Some(katana_grpc::proto::BlockId {
        identifier: Some(katana_grpc::proto::block_id::Identifier::Number(n)),
    })
}

fn grpc_block_id_latest() -> Option<katana_grpc::proto::BlockId> {
    Some(katana_grpc::proto::BlockId {
        identifier: Some(katana_grpc::proto::block_id::Identifier::Tag(BlockTag::Latest as i32)),
    })
}

#[tokio::test]
async fn test_chain_id() {
    let (node, mut grpc, rpc) = setup().await;

    let chain_id = node.backend().chain_spec.id().id();

    let rpc_chain_id = rpc.chain_id().await.expect("rpc chain_id failed");

    let grpc_chain_id = grpc
        .chain_id(Request::new(ChainIdRequest {}))
        .await
        .expect("grpc chain_id failed")
        .into_inner()
        .chain_id;

    assert_eq!(rpc_chain_id, chain_id);
    assert_eq!(grpc_chain_id, format!("{:#x}", chain_id));
}

#[tokio::test]
async fn test_block_number() {
    let (_node, mut grpc, rpc) = setup().await;

    let rpc_block_number = rpc.block_number().await.expect("rpc block_number failed");

    let grpc_block_number = grpc
        .block_number(Request::new(BlockNumberRequest {}))
        .await
        .expect("grpc block_number failed")
        .into_inner()
        .block_number;

    assert_eq!(grpc_block_number, rpc_block_number);
    assert!(rpc_block_number > 0, "Expected block number > 0 after migration");
}

#[tokio::test]
async fn test_block_hash_and_number() {
    let (_node, mut grpc, rpc) = setup().await;

    let rpc_result = rpc.block_hash_and_number().await.expect("rpc block_hash_and_number failed");

    let grpc_result = grpc
        .block_hash_and_number(Request::new(BlockHashAndNumberRequest {}))
        .await
        .expect("grpc block_hash_and_number failed")
        .into_inner();

    assert_eq!(grpc_result.block_number, rpc_result.block_number);
    let grpc_hash = proto_to_felt(&grpc_result.block_hash.expect("grpc missing block_hash"));
    assert_eq!(grpc_hash, rpc_result.block_hash);
}

#[tokio::test]
async fn test_get_block_with_txs() {
    let (_node, mut grpc, rpc) = setup().await;

    let rpc_block = rpc.get_block_with_txs(BlockId::Number(0)).await.expect("rpc failed");

    let grpc_result = grpc
        .get_block_with_txs(Request::new(GetBlockRequest { block_id: grpc_block_id_number(0) }))
        .await
        .expect("grpc failed")
        .into_inner();

    let rpc_block = match rpc_block {
        starknet::core::types::MaybePreConfirmedBlockWithTxs::Block(b) => b,
        _ => panic!("Expected confirmed block from rpc"),
    };

    let grpc_block = match grpc_result.result {
        Some(katana_grpc::proto::get_block_with_txs_response::Result::Block(b)) => b,
        _ => panic!("Expected confirmed block from grpc"),
    };

    let grpc_header = grpc_block.header.expect("grpc missing block header");
    assert_eq!(grpc_header.block_number, rpc_block.block_number);
    assert_eq!(
        proto_to_felt(&grpc_header.block_hash.expect("grpc missing block_hash")),
        rpc_block.block_hash
    );
    assert_eq!(grpc_block.transactions.len(), rpc_block.transactions.len());
}

#[tokio::test]
async fn test_get_block_with_tx_hashes() {
    let (_node, mut grpc, rpc) = setup().await;

    let rpc_block = rpc.get_block_with_tx_hashes(BlockId::Number(0)).await.expect("rpc failed");

    let grpc_result = grpc
        .get_block_with_tx_hashes(Request::new(GetBlockRequest {
            block_id: grpc_block_id_number(0),
        }))
        .await
        .expect("grpc failed")
        .into_inner();

    let rpc_block = match rpc_block {
        starknet::core::types::MaybePreConfirmedBlockWithTxHashes::Block(b) => b,
        _ => panic!("Expected confirmed block from rpc"),
    };

    let grpc_block = match grpc_result.result {
        Some(katana_grpc::proto::get_block_with_tx_hashes_response::Result::Block(b)) => b,
        _ => panic!("Expected confirmed block from grpc"),
    };

    let grpc_header = grpc_block.header.expect("grpc missing block header");
    assert_eq!(grpc_header.block_number, rpc_block.block_number);
    assert_eq!(
        proto_to_felt(&grpc_header.block_hash.expect("grpc missing block_hash")),
        rpc_block.block_hash
    );
    assert_eq!(grpc_block.transactions.len(), rpc_block.transactions.len());

    for (grpc_tx_hash, rpc_tx_hash) in
        grpc_block.transactions.iter().zip(rpc_block.transactions.iter())
    {
        assert_eq!(proto_to_felt(grpc_tx_hash), *rpc_tx_hash);
    }
}

#[tokio::test]
async fn test_get_block_with_txs_latest() {
    let (_node, mut grpc, rpc) = setup().await;

    let rpc_block =
        rpc.get_block_with_txs(BlockId::Tag(StarknetBlockTag::Latest)).await.expect("rpc failed");

    let grpc_result = grpc
        .get_block_with_txs(Request::new(GetBlockRequest { block_id: grpc_block_id_latest() }))
        .await
        .expect("grpc failed")
        .into_inner();

    let rpc_block = match rpc_block {
        starknet::core::types::MaybePreConfirmedBlockWithTxs::Block(b) => b,
        _ => panic!("Expected confirmed block from rpc"),
    };

    let grpc_block = match grpc_result.result {
        Some(katana_grpc::proto::get_block_with_txs_response::Result::Block(b)) => b,
        _ => panic!("Expected confirmed block from grpc"),
    };

    let grpc_header = grpc_block.header.expect("grpc missing block header");
    assert_eq!(grpc_header.block_number, rpc_block.block_number);
    assert_eq!(grpc_block.transactions.len(), rpc_block.transactions.len());
}

#[tokio::test]
async fn test_get_class_at() {
    let (node, mut grpc, rpc) = setup().await;
    let address = genesis_address(&node);

    let rpc_class = rpc
        .get_class_at(BlockId::Tag(StarknetBlockTag::Latest), address)
        .await
        .expect("rpc get_class_at failed");

    let grpc_result = grpc
        .get_class_at(Request::new(GetClassAtRequest {
            block_id: grpc_block_id_latest(),
            contract_address: Some(felt_to_proto(address)),
        }))
        .await
        .expect("grpc get_class_at failed")
        .into_inner();

    // Verify both return a Sierra class (not Legacy)
    assert!(
        matches!(rpc_class, starknet::core::types::ContractClass::Sierra(_)),
        "Expected Sierra class from rpc"
    );
    assert!(
        matches!(
            grpc_result.result,
            Some(katana_grpc::proto::get_class_at_response::Result::ContractClass(_))
        ),
        "Expected Sierra class from grpc"
    );
}

#[tokio::test]
async fn test_get_class_hash_at() {
    let (node, mut grpc, rpc) = setup().await;
    let address = genesis_address(&node);

    let rpc_class_hash = rpc
        .get_class_hash_at(BlockId::Tag(StarknetBlockTag::Latest), address)
        .await
        .expect("rpc get_class_hash_at failed");

    let grpc_result = grpc
        .get_class_hash_at(Request::new(GetClassHashAtRequest {
            block_id: grpc_block_id_latest(),
            contract_address: Some(felt_to_proto(address)),
        }))
        .await
        .expect("grpc get_class_hash_at failed")
        .into_inner();

    let grpc_class_hash = proto_to_felt(&grpc_result.class_hash.expect("grpc missing class_hash"));
    assert_eq!(grpc_class_hash, rpc_class_hash);
}

#[tokio::test]
async fn test_get_storage_at() {
    let (node, mut grpc, rpc) = setup().await;
    let address = genesis_address(&node);

    let rpc_value = rpc
        .get_storage_at(address, Felt::ZERO, BlockId::Tag(StarknetBlockTag::Latest))
        .await
        .expect("rpc get_storage_at failed");

    let grpc_result = grpc
        .get_storage_at(Request::new(GetStorageAtRequest {
            block_id: grpc_block_id_latest(),
            contract_address: Some(felt_to_proto(address)),
            key: Some(felt_to_proto(Felt::ZERO)),
        }))
        .await
        .expect("grpc get_storage_at failed")
        .into_inner();

    let grpc_value = proto_to_felt(&grpc_result.value.expect("grpc missing value"));
    assert_eq!(grpc_value, rpc_value);
}

#[tokio::test]
async fn test_get_nonce() {
    let (node, mut grpc, rpc) = setup().await;
    let address = genesis_address(&node);

    let rpc_nonce = rpc
        .get_nonce(BlockId::Tag(StarknetBlockTag::Latest), address)
        .await
        .expect("rpc get_nonce failed");

    let grpc_result = grpc
        .get_nonce(Request::new(GetNonceRequest {
            block_id: grpc_block_id_latest(),
            contract_address: Some(felt_to_proto(address)),
        }))
        .await
        .expect("grpc get_nonce failed")
        .into_inner();

    let grpc_nonce = proto_to_felt(&grpc_result.nonce.expect("grpc missing nonce"));
    assert_eq!(grpc_nonce, rpc_nonce);
    assert!(rpc_nonce > Felt::ZERO, "Nonce should be > 0 after migration");
}

#[tokio::test]
async fn test_spec_version() {
    let (_node, mut grpc, rpc) = setup().await;

    let rpc_version = rpc.spec_version().await.expect("rpc spec_version failed");

    let grpc_version = grpc
        .spec_version(Request::new(SpecVersionRequest {}))
        .await
        .expect("grpc spec_version failed")
        .into_inner()
        .version;

    assert_eq!(grpc_version, rpc_version);
}

#[tokio::test]
async fn test_syncing() {
    let (_node, mut grpc, rpc) = setup().await;

    let rpc_syncing = rpc.syncing().await.expect("rpc syncing failed");

    let grpc_syncing = grpc
        .syncing(Request::new(SyncingRequest {}))
        .await
        .expect("grpc syncing failed")
        .into_inner();

    assert!(
        matches!(rpc_syncing, starknet::core::types::SyncStatusType::NotSyncing),
        "Expected rpc to report not syncing"
    );
    assert!(
        matches!(
            grpc_syncing.result,
            Some(katana_grpc::proto::syncing_response::Result::NotSyncing(true))
        ),
        "Expected grpc to report not syncing"
    );
}

#[tokio::test]
async fn test_get_block_transaction_count() {
    let (_node, mut grpc, rpc) = setup().await;

    let rpc_count = rpc
        .get_block_transaction_count(BlockId::Number(0))
        .await
        .expect("rpc get_block_transaction_count failed");

    let grpc_count = grpc
        .get_block_transaction_count(Request::new(GetBlockRequest {
            block_id: grpc_block_id_number(0),
        }))
        .await
        .expect("grpc get_block_transaction_count failed")
        .into_inner()
        .count;

    assert_eq!(grpc_count, rpc_count);
}

#[tokio::test]
async fn test_get_state_update() {
    let (_node, mut grpc, rpc) = setup().await;

    let rpc_state =
        rpc.get_state_update(BlockId::Number(0)).await.expect("rpc get_state_update failed");

    let grpc_result = grpc
        .get_state_update(Request::new(GetBlockRequest { block_id: grpc_block_id_number(0) }))
        .await
        .expect("grpc get_state_update failed")
        .into_inner();

    let rpc_state = match rpc_state {
        starknet::core::types::MaybePreConfirmedStateUpdate::Update(s) => s,
        _ => panic!("Expected confirmed state update from rpc"),
    };

    let grpc_state = match grpc_result.result {
        Some(katana_grpc::proto::get_state_update_response::Result::StateUpdate(s)) => s,
        _ => panic!("Expected confirmed state update from grpc"),
    };

    assert_eq!(
        proto_to_felt(&grpc_state.block_hash.expect("grpc missing block_hash")),
        rpc_state.block_hash
    );
    assert_eq!(
        proto_to_felt(&grpc_state.new_root.expect("grpc missing new_root")),
        rpc_state.new_root
    );
    assert_eq!(
        proto_to_felt(&grpc_state.old_root.expect("grpc missing old_root")),
        rpc_state.old_root
    );
}

#[tokio::test]
async fn test_get_events() {
    let (_node, mut grpc, rpc) = setup().await;

    let rpc_events = rpc
        .get_events(
            EventFilter {
                from_block: Some(BlockId::Number(0)),
                to_block: Some(BlockId::Tag(StarknetBlockTag::Latest)),
                address: None,
                keys: None,
            },
            None,
            100,
        )
        .await
        .expect("rpc get_events failed");

    let grpc_events = grpc
        .get_events(Request::new(GetEventsRequest {
            filter: Some(katana_grpc::proto::EventFilter {
                from_block: grpc_block_id_number(0),
                to_block: grpc_block_id_latest(),
                address: None,
                keys: vec![],
            }),
            chunk_size: 100,
            continuation_token: String::new(),
        }))
        .await
        .expect("grpc get_events failed")
        .into_inner();

    assert_eq!(grpc_events.events.len(), rpc_events.events.len());
    assert!(!rpc_events.events.is_empty(), "Expected events after migration");

    let rpc_event = &rpc_events.events[0];
    let grpc_event = &grpc_events.events[0];

    assert_eq!(
        proto_to_felt(grpc_event.from_address.as_ref().expect("grpc missing from_address")),
        rpc_event.from_address
    );
    assert_eq!(
        proto_to_felt(grpc_event.transaction_hash.as_ref().expect("grpc missing tx_hash")),
        rpc_event.transaction_hash
    );
    assert_eq!(grpc_event.keys.len(), rpc_event.keys.len());
    assert_eq!(grpc_event.data.len(), rpc_event.data.len());
}

#[tokio::test]
async fn test_get_transaction_by_hash() {
    let (_node, mut grpc, rpc) = setup().await;

    // Get a transaction hash from block 1
    let rpc_block =
        rpc.get_block_with_tx_hashes(BlockId::Number(1)).await.expect("rpc get_block failed");

    let tx_hash = match rpc_block {
        starknet::core::types::MaybePreConfirmedBlockWithTxHashes::Block(b) => {
            *b.transactions.first().expect("no transactions in block 1")
        }
        _ => panic!("Expected confirmed block"),
    };

    // Fetch via JSON-RPC
    let rpc_tx =
        rpc.get_transaction_by_hash(tx_hash).await.expect("rpc get_transaction_by_hash failed");

    // Fetch via gRPC
    let grpc_result = grpc
        .get_transaction_by_hash(Request::new(GetTransactionByHashRequest {
            transaction_hash: Some(felt_to_proto(tx_hash)),
        }))
        .await
        .expect("grpc get_transaction_by_hash failed")
        .into_inner();

    // Verify RPC returned the correct hash
    assert_eq!(*rpc_tx.transaction_hash(), tx_hash);

    // Verify gRPC returned a valid transaction
    let grpc_tx = grpc_result.transaction.expect("grpc missing transaction");
    assert!(grpc_tx.transaction.is_some(), "grpc transaction should have a variant");
}

#[tokio::test]
async fn test_get_transaction_receipt() {
    let (_node, mut grpc, rpc) = setup().await;

    // Get a transaction hash from block 1
    let rpc_block =
        rpc.get_block_with_tx_hashes(BlockId::Number(1)).await.expect("rpc get_block failed");

    let tx_hash = match rpc_block {
        starknet::core::types::MaybePreConfirmedBlockWithTxHashes::Block(b) => {
            *b.transactions.first().expect("no transactions in block 1")
        }
        _ => panic!("Expected confirmed block"),
    };

    // Fetch receipt via JSON-RPC
    let rpc_receipt =
        rpc.get_transaction_receipt(tx_hash).await.expect("rpc get_transaction_receipt failed");

    // Fetch receipt via gRPC
    let grpc_result = grpc
        .get_transaction_receipt(Request::new(GetTransactionReceiptRequest {
            transaction_hash: Some(felt_to_proto(tx_hash)),
        }))
        .await
        .expect("grpc get_transaction_receipt failed")
        .into_inner();

    let grpc_receipt = grpc_result.receipt.expect("grpc missing receipt");

    // Compare transaction hash
    assert_eq!(*rpc_receipt.receipt.transaction_hash(), tx_hash);
    assert_eq!(
        proto_to_felt(&grpc_receipt.transaction_hash.expect("grpc missing tx_hash")),
        tx_hash
    );

    // Compare block number
    assert_eq!(grpc_receipt.block_number, rpc_receipt.block.block_number());
}
