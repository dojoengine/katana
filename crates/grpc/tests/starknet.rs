use katana_grpc::proto::{
    BlockHashAndNumberRequest, BlockNumberRequest, BlockTag, CallRequest, ChainIdRequest,
    EstimateFeeRequest, EstimateMessageFeeRequest, GetBlockRequest, GetClassAtRequest,
    GetClassHashAtRequest, GetClassRequest, GetCompiledCasmRequest, GetEventsRequest,
    GetNonceRequest, GetStorageAtRequest, GetStorageProofRequest,
    GetTransactionByBlockIdAndIndexRequest, GetTransactionByHashRequest,
    GetTransactionReceiptRequest, GetTransactionStatusRequest, SpecVersionRequest, SyncingRequest,
};
use katana_grpc::GrpcClient;
use katana_primitives::block::BlockIdOrTag;
use katana_primitives::{ContractAddress, Felt};
use katana_rpc_types::block::{
    GetBlockWithReceiptsResponse, GetBlockWithTxHashesResponse, MaybePreConfirmedBlock,
};
use katana_rpc_types::state_update::StateUpdate;
use katana_rpc_types::transaction::{RpcTx, TxStatus};
use katana_rpc_types::{EventFilter, ExecutionResult, FunctionCall, SyncingResponse};
use katana_starknet::rpc::StarknetRpcClient;
use katana_utils::node::TestNode;
use starknet::core::utils::get_selector_from_name;
use tonic::{Code, Request};

async fn setup() -> (TestNode, GrpcClient, StarknetRpcClient) {
    let node = TestNode::new_with_spawn_and_move_db().await;

    let grpc_addr = *node.grpc_addr().expect("grpc not enabled");
    let grpc = GrpcClient::connect(format!("http://{grpc_addr}"))
        .await
        .expect("failed to connect to gRPC server");

    let rpc = node.starknet_rpc_client();

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

fn grpc_block_id_hash(hash: Felt) -> Option<katana_grpc::proto::BlockId> {
    Some(katana_grpc::proto::BlockId {
        identifier: Some(katana_grpc::proto::block_id::Identifier::Hash(felt_to_proto(hash))),
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

    let rpc_block_number = rpc.block_number().await.expect("rpc block_number failed").block_number;

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

    let rpc_block = rpc.get_block_with_txs(BlockIdOrTag::Number(0)).await.expect("rpc failed");

    let grpc_result = grpc
        .get_block_with_txs(Request::new(GetBlockRequest { block_id: grpc_block_id_number(0) }))
        .await
        .expect("grpc failed")
        .into_inner();

    let rpc_block = match rpc_block {
        MaybePreConfirmedBlock::Confirmed(b) => b,
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

    let rpc_block =
        rpc.get_block_with_tx_hashes(BlockIdOrTag::Number(0)).await.expect("rpc failed");

    let grpc_result = grpc
        .get_block_with_tx_hashes(Request::new(GetBlockRequest {
            block_id: grpc_block_id_number(0),
        }))
        .await
        .expect("grpc failed")
        .into_inner();

    let rpc_block = match rpc_block {
        GetBlockWithTxHashesResponse::Block(b) => b,
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

    let rpc_block = rpc.get_block_with_txs(BlockIdOrTag::Latest).await.expect("rpc failed");

    let grpc_result = grpc
        .get_block_with_txs(Request::new(GetBlockRequest { block_id: grpc_block_id_latest() }))
        .await
        .expect("grpc failed")
        .into_inner();

    let rpc_block = match rpc_block {
        MaybePreConfirmedBlock::Confirmed(b) => b,
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
        .get_class_at(BlockIdOrTag::Latest, ContractAddress::from(address))
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
        matches!(rpc_class, katana_rpc_types::class::Class::Sierra(_)),
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
        .get_class_hash_at(BlockIdOrTag::Latest, ContractAddress::from(address))
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
        .get_storage_at(ContractAddress::from(address), Felt::ZERO, BlockIdOrTag::Latest)
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
        .get_nonce(BlockIdOrTag::Latest, ContractAddress::from(address))
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
        matches!(rpc_syncing, SyncingResponse::NotSyncing),
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
        .get_block_transaction_count(BlockIdOrTag::Number(0))
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
        rpc.get_state_update(BlockIdOrTag::Number(0)).await.expect("rpc get_state_update failed");

    let grpc_result = grpc
        .get_state_update(Request::new(GetBlockRequest { block_id: grpc_block_id_number(0) }))
        .await
        .expect("grpc get_state_update failed")
        .into_inner();

    let rpc_state = match rpc_state {
        StateUpdate::Confirmed(s) => s,
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
                from_block: Some(BlockIdOrTag::Number(0)),
                to_block: Some(BlockIdOrTag::Latest),
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
        Felt::from(rpc_event.from_address)
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
        rpc.get_block_with_tx_hashes(BlockIdOrTag::Number(1)).await.expect("rpc get_block failed");

    let tx_hash = match rpc_block {
        GetBlockWithTxHashesResponse::Block(b) => {
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
    assert_eq!(rpc_tx.transaction_hash, tx_hash);

    // Verify gRPC returned a valid transaction
    let grpc_tx = grpc_result.transaction.expect("grpc missing transaction");
    assert!(grpc_tx.transaction.is_some(), "grpc transaction should have a variant");
}

#[tokio::test]
async fn test_get_transaction_receipt() {
    let (_node, mut grpc, rpc) = setup().await;

    // Get a transaction hash from block 1
    let rpc_block =
        rpc.get_block_with_tx_hashes(BlockIdOrTag::Number(1)).await.expect("rpc get_block failed");

    let tx_hash = match rpc_block {
        GetBlockWithTxHashesResponse::Block(b) => {
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
    assert_eq!(rpc_receipt.transaction_hash, tx_hash);
    assert_eq!(
        proto_to_felt(&grpc_receipt.transaction_hash.expect("grpc missing tx_hash")),
        tx_hash
    );

    // Compare block number: the receipt's ReceiptBlockInfo is either Block{block_number, ..}
    // or PreConfirmed{block_number}; pattern match to extract.
    use katana_rpc_types::receipt::ReceiptBlockInfo;
    let rpc_block_number = match rpc_receipt.block {
        ReceiptBlockInfo::Block { block_number, .. } => block_number,
        ReceiptBlockInfo::PreConfirmed { block_number } => block_number,
    };
    assert_eq!(grpc_receipt.block_number, rpc_block_number);
}

#[tokio::test]
async fn test_get_block_with_receipts() {
    let (_node, mut grpc, rpc) = setup().await;

    let rpc_block = rpc
        .get_block_with_receipts(BlockIdOrTag::Number(0))
        .await
        .expect("rpc get_block_with_receipts");

    let grpc_result = grpc
        .get_block_with_receipts(Request::new(GetBlockRequest {
            block_id: grpc_block_id_number(0),
        }))
        .await
        .expect("grpc get_block_with_receipts failed")
        .into_inner();

    let rpc_block = match rpc_block {
        GetBlockWithReceiptsResponse::Block(b) => b,
        _ => panic!("Expected confirmed block from rpc"),
    };

    let grpc_block = match grpc_result.result {
        Some(katana_grpc::proto::get_block_with_receipts_response::Result::Block(b)) => b,
        _ => panic!("Expected confirmed block from grpc"),
    };

    let grpc_header = grpc_block.header.expect("grpc missing header");
    assert_eq!(grpc_header.block_number, rpc_block.block_number);
    assert_eq!(
        proto_to_felt(&grpc_header.block_hash.expect("grpc missing block_hash")),
        rpc_block.block_hash
    );
    assert_eq!(
        proto_to_felt(&grpc_header.parent_hash.expect("grpc missing parent_hash")),
        rpc_block.parent_hash
    );
    assert_eq!(
        proto_to_felt(&grpc_header.new_root.expect("grpc missing new_root")),
        rpc_block.new_root
    );
    assert_eq!(grpc_block.transactions.len(), rpc_block.transactions.len());

    // Element-wise receipt hash parity: a handler that returns empty/garbage receipts
    // but the right length would otherwise pass silently.
    for (grpc_twr, rpc_twr) in grpc_block.transactions.iter().zip(rpc_block.transactions.iter()) {
        let grpc_receipt = grpc_twr.receipt.as_ref().expect("grpc missing receipt");
        let grpc_hash = proto_to_felt(
            grpc_receipt.transaction_hash.as_ref().expect("grpc receipt missing tx_hash"),
        );
        assert_eq!(grpc_hash, rpc_twr.receipt.transaction_hash);
    }
}

#[tokio::test]
async fn test_get_transaction_status() {
    let (_node, mut grpc, rpc) = setup().await;

    let rpc_block =
        rpc.get_block_with_tx_hashes(BlockIdOrTag::Number(1)).await.expect("rpc get_block failed");
    let tx_hash = match rpc_block {
        GetBlockWithTxHashesResponse::Block(b) => {
            *b.transactions.first().expect("no tx in block 1")
        }
        _ => panic!("Expected confirmed block"),
    };

    let rpc_status =
        rpc.get_transaction_status(tx_hash).await.expect("rpc get_transaction_status failed");

    let grpc_resp = grpc
        .get_transaction_status(Request::new(GetTransactionStatusRequest {
            transaction_hash: Some(felt_to_proto(tx_hash)),
        }))
        .await
        .expect("grpc get_transaction_status failed")
        .into_inner();

    let (expected_finality, expected_execution) = match rpc_status {
        TxStatus::Received => ("RECEIVED", ""),
        TxStatus::Candidate => ("CANDIDATE", ""),
        TxStatus::PreConfirmed(ref e) => ("PRE_CONFIRMED", execution_result_str(e)),
        TxStatus::AcceptedOnL2(ref e) => ("ACCEPTED_ON_L2", execution_result_str(e)),
        TxStatus::AcceptedOnL1(ref e) => ("ACCEPTED_ON_L1", execution_result_str(e)),
    };

    assert_eq!(grpc_resp.finality_status, expected_finality);
    assert_eq!(grpc_resp.execution_status, expected_execution);
}

fn execution_result_str(e: &ExecutionResult) -> &'static str {
    match e {
        ExecutionResult::Succeeded => "SUCCEEDED",
        ExecutionResult::Reverted { .. } => "REVERTED",
    }
}

#[tokio::test]
async fn test_get_transaction_by_block_id_and_index() {
    let (_node, mut grpc, rpc) = setup().await;

    let rpc_tx = rpc
        .get_transaction_by_block_id_and_index(BlockIdOrTag::Number(1), 0)
        .await
        .expect("rpc get_transaction_by_block_id_and_index failed");

    let grpc_resp = grpc
        .get_transaction_by_block_id_and_index(Request::new(
            GetTransactionByBlockIdAndIndexRequest { block_id: grpc_block_id_number(1), index: 0 },
        ))
        .await
        .expect("grpc get_transaction_by_block_id_and_index failed")
        .into_inner();

    let grpc_tx = grpc_resp.transaction.expect("grpc missing transaction");
    let grpc_variant = grpc_tx.transaction.as_ref().expect("grpc transaction missing variant");

    // Cross-ref: the tx at (block 1, index 0) matches the first hash in the block's tx list.
    let block = match rpc
        .get_block_with_tx_hashes(BlockIdOrTag::Number(1))
        .await
        .expect("rpc get_block_with_tx_hashes failed")
    {
        GetBlockWithTxHashesResponse::Block(b) => b,
        _ => panic!("Expected confirmed block"),
    };
    assert_eq!(rpc_tx.transaction_hash, *block.transactions.first().expect("no tx at index 0"));

    // Variant-kind parity: handler could otherwise return an arbitrary non-None tx of
    // the wrong kind and this test would pass on `is_some()` alone.
    use katana_grpc::proto::transaction::Transaction as GrpcTxVariant;
    let grpc_kind = match grpc_variant {
        GrpcTxVariant::InvokeV1(_) | GrpcTxVariant::InvokeV3(_) => "invoke",
        GrpcTxVariant::DeclareV1(_) | GrpcTxVariant::DeclareV2(_) | GrpcTxVariant::DeclareV3(_) => {
            "declare"
        }
        GrpcTxVariant::DeployAccount(_) | GrpcTxVariant::DeployAccountV3(_) => "deploy_account",
        GrpcTxVariant::L1Handler(_) => "l1_handler",
        GrpcTxVariant::Deploy(_) => "deploy",
    };
    let rpc_kind = match rpc_tx.transaction {
        RpcTx::Invoke(_) => "invoke",
        RpcTx::Declare(_) => "declare",
        RpcTx::DeployAccount(_) => "deploy_account",
        RpcTx::L1Handler(_) => "l1_handler",
        RpcTx::Deploy(_) => "deploy",
    };
    assert_eq!(grpc_kind, rpc_kind, "grpc tx variant kind should match rpc");
}

#[tokio::test]
async fn test_get_class() {
    let (node, mut grpc, rpc) = setup().await;
    let address = genesis_address(&node);

    let class_hash = rpc
        .get_class_hash_at(BlockIdOrTag::Latest, ContractAddress::from(address))
        .await
        .expect("rpc get_class_hash_at failed");

    let grpc_resp = grpc
        .get_class(Request::new(GetClassRequest {
            block_id: grpc_block_id_latest(),
            class_hash: Some(felt_to_proto(class_hash)),
        }))
        .await
        .expect("grpc get_class failed")
        .into_inner();

    // NOTE: handler is lossy — it serializes the whole class to JSON into the `abi`
    // field and leaves sierra_program/entry_points_by_type empty. This assertion
    // verifies the Sierra variant is returned AND the abi field is valid, non-trivial
    // JSON (catches silent `unwrap_or_default` empty-string failures). When the
    // handler is rewritten to populate sierra_program properly, strengthen this test.
    match grpc_resp.result {
        Some(katana_grpc::proto::get_class_response::Result::ContractClass(cc)) => {
            let parsed: serde_json::Value =
                serde_json::from_str(&cc.abi).expect("abi must parse as JSON");
            let obj = parsed.as_object().expect("expected JSON object (class shape)");
            // A Sierra class JSON must contain at least one of these top-level keys.
            // Guards against the handler returning `{}` via `unwrap_or_default` or
            // stuffing unrelated JSON into the abi field.
            let has_class_shape = obj.contains_key("sierra_program")
                || obj.contains_key("abi")
                || obj.contains_key("entry_points_by_type");
            assert!(
                has_class_shape,
                "abi JSON must be class-shaped (sierra_program/abi/entry_points_by_type), got \
                 keys: {:?}",
                obj.keys().collect::<Vec<_>>()
            );
        }
        other => panic!("expected ContractClass variant, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_call() {
    let (node, mut grpc, rpc) = setup().await;
    let genesis = genesis_address(&node);

    // STRK fee token address (katana_genesis::constant::DEFAULT_STRK_FEE_TOKEN_ADDRESS)
    let strk_token = Felt::from_hex_unchecked(
        "0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d",
    );
    let selector = get_selector_from_name("balanceOf").expect("selector");

    let rpc_result = rpc
        .call(
            FunctionCall {
                contract_address: ContractAddress::from(strk_token),
                entry_point_selector: selector,
                calldata: vec![genesis],
            },
            BlockIdOrTag::Latest,
        )
        .await
        .expect("rpc call failed")
        .result;

    let grpc_resp = grpc
        .call(Request::new(CallRequest {
            block_id: grpc_block_id_latest(),
            request: Some(katana_grpc::proto::FunctionCall {
                contract_address: Some(felt_to_proto(strk_token)),
                entry_point_selector: Some(felt_to_proto(selector)),
                calldata: vec![felt_to_proto(genesis)],
            }),
        }))
        .await
        .expect("grpc call failed")
        .into_inner();

    assert_eq!(grpc_resp.result.len(), rpc_result.len());
    for (g, r) in grpc_resp.result.iter().zip(rpc_result.iter()) {
        assert_eq!(proto_to_felt(g), *r);
    }
    // balanceOf returns a u256 = 2 felts (low, high); genesis is prefunded so low should be > 0.
    assert!(
        rpc_result.len() >= 2,
        "balanceOf should return at least 2 felts, got {}",
        rpc_result.len()
    );
    assert_ne!(rpc_result[0], Felt::ZERO, "prefunded genesis balance low should be non-zero");
}

#[tokio::test]
async fn test_get_storage_proof() {
    let (node, mut grpc, _rpc) = setup().await;
    let address = genesis_address(&node);

    // Cross-reference: the proof's block_hash should match the latest block.
    let bh = grpc
        .block_hash_and_number(Request::new(BlockHashAndNumberRequest {}))
        .await
        .expect("block_hash_and_number failed")
        .into_inner();
    let latest_hash = proto_to_felt(&bh.block_hash.expect("missing block_hash"));

    let resp = grpc
        .get_storage_proof(Request::new(GetStorageProofRequest {
            block_id: grpc_block_id_latest(),
            class_hashes: vec![],
            contract_addresses: vec![felt_to_proto(address)],
            contracts_storage_keys: vec![],
        }))
        .await
        .expect("grpc get_storage_proof failed")
        .into_inner();

    let proof = resp.proof.expect("missing proof");
    let roots = proof.global_roots.expect("missing global_roots");
    assert_eq!(
        proto_to_felt(&roots.block_hash.expect("missing roots.block_hash")),
        latest_hash,
        "storage proof block_hash should match latest block"
    );
    assert_ne!(
        proto_to_felt(&roots.contracts_tree_root.expect("missing contracts_tree_root")),
        Felt::ZERO,
        "contracts_tree_root should be non-zero on a migrated chain"
    );

    let contracts_proof = proof.contracts_proof.expect("missing contracts_proof");
    assert_eq!(
        contracts_proof.contract_leaves_data.len(),
        1,
        "expected one contract leaf for the requested address"
    );
}

#[tokio::test]
async fn test_estimate_fee_unimplemented() {
    let (_node, mut grpc, _rpc) = setup().await;

    let status = grpc
        .estimate_fee(Request::new(EstimateFeeRequest {
            transactions: vec![],
            simulation_flags: vec![],
            block_id: grpc_block_id_latest(),
        }))
        .await
        .expect_err("estimate_fee should return Unimplemented");

    assert_eq!(
        status.code(),
        Code::Unimplemented,
        "expected Unimplemented, got: {:?}",
        status.code()
    );
}

#[tokio::test]
async fn test_estimate_message_fee_unimplemented() {
    let (_node, mut grpc, _rpc) = setup().await;

    let status = grpc
        .estimate_message_fee(Request::new(EstimateMessageFeeRequest {
            message: None,
            block_id: grpc_block_id_latest(),
        }))
        .await
        .expect_err("estimate_message_fee should return Unimplemented");

    assert_eq!(status.code(), Code::Unimplemented);
}

#[tokio::test]
async fn test_get_compiled_casm_unimplemented() {
    let (_node, mut grpc, _rpc) = setup().await;

    let status = grpc
        .get_compiled_casm(Request::new(GetCompiledCasmRequest {
            class_hash: Some(felt_to_proto(Felt::ZERO)),
        }))
        .await
        .expect_err("get_compiled_casm should return Unimplemented");

    assert_eq!(status.code(), Code::Unimplemented);
}

#[tokio::test]
async fn test_get_events_pagination() {
    let (_node, mut grpc, _rpc) = setup().await;

    // Baseline: fetch with the largest chunk the server accepts (100 is the server cap;
    // larger chunks return ResourceExhausted). A dev chain has well under 100 events
    // so this is effectively "fetch all" for comparison purposes.
    let baseline = grpc
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
        .expect("baseline get_events failed")
        .into_inner();

    assert!(
        baseline.events.len() >= 4,
        "need at least 4 events to exercise pagination, got {}",
        baseline.events.len()
    );
    assert!(baseline.continuation_token.is_empty(), "baseline fetch should be the final page");

    // Walk the same query with chunk_size = 2 and compare order+length.
    let mut paginated = Vec::new();
    let mut token = String::new();
    let mut guard = 0;
    loop {
        let resp = grpc
            .get_events(Request::new(GetEventsRequest {
                filter: Some(katana_grpc::proto::EventFilter {
                    from_block: grpc_block_id_number(0),
                    to_block: grpc_block_id_latest(),
                    address: None,
                    keys: vec![],
                }),
                chunk_size: 2,
                continuation_token: token.clone(),
            }))
            .await
            .expect("paginated get_events failed")
            .into_inner();

        paginated.extend(resp.events);
        if resp.continuation_token.is_empty() {
            break;
        }
        token = resp.continuation_token;

        guard += 1;
        assert!(guard < 1000, "pagination did not terminate");
    }

    assert_eq!(
        paginated.len(),
        baseline.events.len(),
        "paginated event count should match baseline"
    );
    for (p, b) in paginated.iter().zip(baseline.events.iter()) {
        assert_eq!(p.transaction_hash, b.transaction_hash);
        assert_eq!(p.block_number, b.block_number);
        assert_eq!(p.keys.len(), b.keys.len());
        assert_eq!(p.data.len(), b.data.len());
    }
}

#[tokio::test]
async fn test_get_block_with_txs_by_hash() {
    let (_node, mut grpc, rpc) = setup().await;

    let bh = grpc
        .block_hash_and_number(Request::new(BlockHashAndNumberRequest {}))
        .await
        .expect("block_hash_and_number failed")
        .into_inner();
    let hash = proto_to_felt(&bh.block_hash.expect("missing block_hash"));
    let number = bh.block_number;

    let rpc_block = rpc
        .get_block_with_txs(BlockIdOrTag::Hash(hash))
        .await
        .expect("rpc get_block_with_txs failed");

    let grpc_result = grpc
        .get_block_with_txs(Request::new(GetBlockRequest { block_id: grpc_block_id_hash(hash) }))
        .await
        .expect("grpc get_block_with_txs failed")
        .into_inner();

    let rpc_block = match rpc_block {
        MaybePreConfirmedBlock::Confirmed(b) => b,
        _ => panic!("Expected confirmed block from rpc"),
    };
    let grpc_block = match grpc_result.result {
        Some(katana_grpc::proto::get_block_with_txs_response::Result::Block(b)) => b,
        _ => panic!("Expected confirmed block from grpc"),
    };

    let grpc_header = grpc_block.header.expect("grpc missing header");
    assert_eq!(grpc_header.block_number, rpc_block.block_number);
    assert_eq!(grpc_header.block_number, number);
    assert_eq!(
        proto_to_felt(&grpc_header.block_hash.expect("grpc missing block_hash")),
        hash,
        "hash-fetched block should have the requested hash"
    );
    // Header-integrity parity: without these, a handler that routed Hash -> Number
    // or returned a different-but-same-length-txs block could pass silently.
    assert_eq!(
        proto_to_felt(&grpc_header.parent_hash.expect("grpc missing parent_hash")),
        rpc_block.parent_hash
    );
    assert_eq!(
        proto_to_felt(&grpc_header.new_root.expect("grpc missing new_root")),
        rpc_block.new_root
    );
    assert_eq!(grpc_block.transactions.len(), rpc_block.transactions.len());
}
