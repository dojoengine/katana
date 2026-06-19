//! Payload (de)serialization benchmark: JSON-RPC vs gRPC.
//!
//! The goal is to isolate how much time each transport spends turning bytes on the wire into the
//! domain types the handlers actually operate on (request parsing), and turning a domain response
//! back into bytes (response serialization). Both transports converge on the *same* domain types
//! (e.g. [`BroadcastedInvokeTx`]), so this is an apples-to-apples comparison of the payload codec:
//!
//! * JSON-RPC parses with `serde_json` straight into the domain type.
//! * gRPC decodes the protobuf message with `prost`, then runs the proto -> domain conversion (the
//!   `TryFrom`/`From` impls in `katana_grpc`'s conversion module) that the handlers invoke.
//!
//! No network, no execution, no server: just the codec boundary. Coverage:
//! * All three write transactions: `add_invoke`, `add_declare` (heavy — a real Sierra class), and
//!   `add_deploy_account` — request parse + response serialize.
//! * `get_storage_at` as a tiny read baseline.
//! * Two heavy reads for the response side: `get_block_with_txs` and `get_transaction_receipt`.
//!
//! # Data
//!
//! Fixtures are generated with random field elements every run, so no single hand-picked payload
//! biases the result. Leaf vectors (calldata, signatures, event keys/data, message payloads, …)
//! get `0..=MAX_VEC_LEN` random elements; the two deliberately *heavy* knobs — the real Sierra
//! class in `declare` and the `BLOCK_TX_COUNT`-tx block — are left at their large sizes. Each
//! method's JSON and gRPC bytes are derived from the *same* random domain object, keeping the two
//! codecs fed equivalent content. Set `BENCH_SEED=<u64>` for a reproducible run.

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use katana_grpc::proto;
use katana_primitives::block::{BlockIdOrTag, FinalityStatus};
use katana_primitives::class::ContractClass;
use katana_primitives::da::{DataAvailabilityMode, L1DataAvailabilityMode};
use katana_primitives::fee::{
    AllResourceBoundsMapping, PriceUnit, ResourceBounds, ResourceBoundsMapping, Tip,
};
use katana_primitives::receipt::{Event, MessageToL1};
use katana_primitives::transaction::{InvokeTx, InvokeTxV1, Tx, TxWithHash};
use katana_primitives::{ContractAddress, Felt};
use katana_rpc_types::block::{BlockWithTxs, MaybePreConfirmedBlock};
use katana_rpc_types::broadcasted::{
    AddDeclareTransactionResponse, AddDeployAccountTransactionResponse,
    AddInvokeTransactionResponse, BroadcastedDeclareTx, BroadcastedDeployAccountTx,
    BroadcastedInvokeTx,
};
use katana_rpc_types::class::RpcSierraContractClass;
use katana_rpc_types::receipt::{
    ExecutionResources, ExecutionResult, FeePayment, ReceiptBlockInfo, RpcInvokeTxReceipt,
    RpcTxReceipt, TxReceiptWithBlockInfo,
};
use katana_rpc_types::transaction::RpcTxWithHash;
use prost::Message;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use starknet::core::types::ResourcePrice;

/// Number of transactions in the synthetic `get_block_with_txs` response. The heavy knob for the
/// block read — deliberately large and *not* subject to the leaf-vector cap.
const BLOCK_TX_COUNT: u64 = 100;

/// Inclusive upper bound on the length of randomly generated *leaf* vectors (calldata, signatures,
/// event keys/data, message payloads, …). Keeps each vector small and realistic while its contents
/// vary every run.
const MAX_VEC_LEN: usize = 10;

/// A real, sizeable Sierra class used as the `DECLARE` payload (~2k felt program). The heavy knob
/// for `declare`; intentionally left intact (not capped to `MAX_VEC_LEN`).
const DECLARE_CLASS_JSON: &str =
    include_str!("../../contracts/build/katana_account_Account.contract_class.json");

/// Build the benchmark RNG: fresh OS entropy per process so every run exercises unique data
/// (avoiding fixture bias). Set `BENCH_SEED=<u64>` to pin the data for a reproducible run.
fn make_rng() -> StdRng {
    match std::env::var("BENCH_SEED") {
        Ok(seed) => StdRng::seed_from_u64(seed.parse().expect("BENCH_SEED must be a u64")),
        Err(_) => StdRng::from_entropy(),
    }
}

/// A random field element.
fn rand_felt(rng: &mut impl Rng) -> Felt {
    Felt::from(rng.gen::<u128>())
}

/// A random `Vec<Felt>` with `0..=MAX_VEC_LEN` elements.
fn rand_felts(rng: &mut impl Rng) -> Vec<Felt> {
    (0..rng.gen_range(0..=MAX_VEC_LEN)).map(|_| rand_felt(rng)).collect()
}

fn rand_bounds(rng: &mut impl Rng) -> ResourceBounds {
    ResourceBounds { max_amount: rng.gen(), max_price_per_unit: rng.gen() }
}

fn rand_resource_bounds(rng: &mut impl Rng) -> ResourceBoundsMapping {
    ResourceBoundsMapping::All(AllResourceBoundsMapping {
        l1_gas: rand_bounds(rng),
        l2_gas: rand_bounds(rng),
        l1_data_gas: rand_bounds(rng),
    })
}

fn invoke_domain(rng: &mut impl Rng) -> BroadcastedInvokeTx {
    BroadcastedInvokeTx {
        sender_address: ContractAddress::from(rand_felt(rng)),
        calldata: rand_felts(rng),
        signature: rand_felts(rng),
        nonce: rand_felt(rng),
        paymaster_data: rand_felts(rng),
        tip: Tip::from(rng.gen::<u64>()),
        account_deployment_data: rand_felts(rng),
        resource_bounds: rand_resource_bounds(rng),
        fee_data_availability_mode: DataAvailabilityMode::L1,
        nonce_data_availability_mode: DataAvailabilityMode::L1,
        is_query: false,
    }
}

fn declare_domain(rng: &mut impl Rng) -> BroadcastedDeclareTx {
    let class = ContractClass::from_str(DECLARE_CLASS_JSON).expect("invalid class fixture");
    let sierra = match class {
        ContractClass::Class(sierra) => sierra,
        ContractClass::Legacy(_) => panic!("expected a Sierra class fixture"),
    };

    BroadcastedDeclareTx {
        // Heavy knob: the real Sierra class, kept intact.
        contract_class: Arc::new(RpcSierraContractClass::from(sierra)),
        sender_address: ContractAddress::from(rand_felt(rng)),
        compiled_class_hash: rand_felt(rng),
        nonce: rand_felt(rng),
        signature: rand_felts(rng),
        paymaster_data: rand_felts(rng),
        account_deployment_data: rand_felts(rng),
        tip: Tip::from(rng.gen::<u64>()),
        resource_bounds: rand_resource_bounds(rng),
        fee_data_availability_mode: DataAvailabilityMode::L1,
        nonce_data_availability_mode: DataAvailabilityMode::L1,
        is_query: false,
    }
}

fn deploy_account_domain(rng: &mut impl Rng) -> BroadcastedDeployAccountTx {
    BroadcastedDeployAccountTx {
        signature: rand_felts(rng),
        nonce: rand_felt(rng),
        contract_address_salt: rand_felt(rng),
        constructor_calldata: rand_felts(rng),
        class_hash: rand_felt(rng),
        paymaster_data: rand_felts(rng),
        tip: Tip::from(rng.gen::<u64>()),
        resource_bounds: rand_resource_bounds(rng),
        fee_data_availability_mode: DataAvailabilityMode::L1,
        nonce_data_availability_mode: DataAvailabilityMode::L1,
        is_query: false,
    }
}

/// A confirmed block carrying `BLOCK_TX_COUNT` invoke transactions — the heavy read response.
/// Each transaction's leaf vectors (calldata, signature) are randomized within `MAX_VEC_LEN`.
fn block_response(rng: &mut impl Rng) -> MaybePreConfirmedBlock {
    let transactions = (0..BLOCK_TX_COUNT)
        .map(|_| {
            let prim = TxWithHash {
                hash: rand_felt(rng),
                transaction: Tx::Invoke(InvokeTx::V1(InvokeTxV1 {
                    sender_address: ContractAddress::from(rand_felt(rng)),
                    nonce: rand_felt(rng),
                    calldata: rand_felts(rng),
                    signature: rand_felts(rng),
                    max_fee: rng.gen(),
                    ..Default::default()
                })),
            };
            // Reuses the production `From<TxWithHash>` impl (rpc-types/src/transaction.rs:322).
            RpcTxWithHash::from(prim)
        })
        .collect::<Vec<_>>();

    let price = ResourcePrice { price_in_wei: rand_felt(rng), price_in_fri: rand_felt(rng) };
    MaybePreConfirmedBlock::Confirmed(BlockWithTxs {
        status: FinalityStatus::AcceptedOnL2,
        block_hash: rand_felt(rng),
        parent_hash: rand_felt(rng),
        block_number: rng.gen(),
        new_root: rand_felt(rng),
        timestamp: rng.gen(),
        sequencer_address: ContractAddress::from(rand_felt(rng)),
        l1_gas_price: price.clone(),
        l2_gas_price: price.clone(),
        l1_data_gas_price: price,
        l1_da_mode: L1DataAvailabilityMode::Blob,
        starknet_version: "0.13.0".to_string(),
        event_commitment: rand_felt(rng),
        event_count: rng.gen(),
        receipt_commitment: rand_felt(rng),
        state_diff_commitment: rand_felt(rng),
        state_diff_length: rng.gen(),
        transaction_commitment: rand_felt(rng),
        transaction_count: BLOCK_TX_COUNT as u32,
        transactions,
    })
}

/// An invoke receipt with `0..=MAX_VEC_LEN` events and messages — the medium read response.
fn receipt_response(rng: &mut impl Rng) -> TxReceiptWithBlockInfo {
    let events = (0..rng.gen_range(0..=MAX_VEC_LEN))
        .map(|_| Event {
            from_address: ContractAddress::from(rand_felt(rng)),
            keys: rand_felts(rng),
            data: rand_felts(rng),
        })
        .collect();
    let messages_sent = (0..rng.gen_range(0..=MAX_VEC_LEN))
        .map(|_| MessageToL1 {
            from_address: ContractAddress::from(rand_felt(rng)),
            to_address: rand_felt(rng),
            payload: rand_felts(rng),
        })
        .collect();

    TxReceiptWithBlockInfo {
        transaction_hash: rand_felt(rng),
        receipt: RpcTxReceipt::Invoke(RpcInvokeTxReceipt {
            actual_fee: FeePayment { amount: rand_felt(rng), unit: PriceUnit::Fri },
            finality_status: FinalityStatus::AcceptedOnL2,
            messages_sent,
            events,
            execution_resources: ExecutionResources {
                l1_gas: rng.gen(),
                l1_data_gas: rng.gen(),
                l2_gas: rng.gen(),
            },
            execution_result: ExecutionResult::Succeeded,
        }),
        block: ReceiptBlockInfo::Block { block_hash: rand_felt(rng), block_number: rng.gen() },
    }
}

fn felt_to_proto(felt: Felt) -> proto::Felt {
    proto::Felt { value: felt.to_bytes_be().to_vec() }
}

fn proto_block_id(number: u64) -> proto::BlockId {
    proto::BlockId { identifier: Some(proto::block_id::Identifier::Number(number)) }
}

/// Register a `json` vs `grpc` pair under `group`, reporting throughput against each transport's
/// own payload size (so bytes/sec is meaningful even though the encodings differ in size).
fn pair<J, G>(
    c: &mut Criterion,
    group: &str,
    json_bytes: &[u8],
    grpc_bytes: &[u8],
    mut json_op: J,
    mut grpc_op: G,
) where
    J: FnMut(&[u8]),
    G: FnMut(&[u8]),
{
    let mut g = c.benchmark_group(group);

    g.throughput(Throughput::Bytes(json_bytes.len() as u64));
    g.bench_function("json", |b| b.iter(|| json_op(black_box(json_bytes))));

    g.throughput(Throughput::Bytes(grpc_bytes.len() as u64));
    g.bench_function("grpc", |b| b.iter(|| grpc_op(black_box(grpc_bytes))));

    g.finish();
}

////////////////////////////////////////////////////////////////////////////////
// Request parsing: wire bytes -> domain type
////////////////////////////////////////////////////////////////////////////////

fn bench_parse_invoke(c: &mut Criterion) {
    let domain = invoke_domain(&mut make_rng());
    let json = serde_json::to_vec(&domain).unwrap();
    let grpc =
        proto::AddInvokeTransactionRequest { transaction: Some((&domain).into()) }.encode_to_vec();

    pair(
        c,
        "parse_request/invoke",
        &json,
        &grpc,
        |bytes| {
            let _: BroadcastedInvokeTx = serde_json::from_slice(bytes).unwrap();
        },
        |bytes| {
            let req = proto::AddInvokeTransactionRequest::decode(bytes).unwrap();
            let _: BroadcastedInvokeTx = req.transaction.unwrap().try_into().unwrap();
        },
    );
}

fn bench_parse_declare(c: &mut Criterion) {
    let domain = declare_domain(&mut make_rng());
    let json = serde_json::to_vec(&domain).unwrap();
    let grpc =
        proto::AddDeclareTransactionRequest { transaction: Some((&domain).into()) }.encode_to_vec();

    pair(
        c,
        "parse_request/declare",
        &json,
        &grpc,
        |bytes| {
            let _: BroadcastedDeclareTx = serde_json::from_slice(bytes).unwrap();
        },
        |bytes| {
            let req = proto::AddDeclareTransactionRequest::decode(bytes).unwrap();
            let _: BroadcastedDeclareTx = req.transaction.unwrap().try_into().unwrap();
        },
    );
}

fn bench_parse_deploy_account(c: &mut Criterion) {
    let domain = deploy_account_domain(&mut make_rng());
    let json = serde_json::to_vec(&domain).unwrap();
    let grpc = proto::AddDeployAccountTransactionRequest { transaction: Some((&domain).into()) }
        .encode_to_vec();

    pair(
        c,
        "parse_request/deploy_account",
        &json,
        &grpc,
        |bytes| {
            let _: BroadcastedDeployAccountTx = serde_json::from_slice(bytes).unwrap();
        },
        |bytes| {
            let req = proto::AddDeployAccountTransactionRequest::decode(bytes).unwrap();
            let _: BroadcastedDeployAccountTx = req.transaction.unwrap().try_into().unwrap();
        },
    );
}

fn bench_parse_get_storage_at(c: &mut Criterion) {
    let mut rng = make_rng();
    let contract_address = ContractAddress::from(rand_felt(&mut rng));
    let key = rand_felt(&mut rng);
    let block_number = rng.gen::<u64>();
    let block_id = BlockIdOrTag::Number(block_number);

    // JSON-RPC `params` is the positional array `[contract_address, key, block_id]`.
    let json = serde_json::to_vec(&(contract_address, key, block_id)).unwrap();
    let grpc = proto::GetStorageAtRequest {
        block_id: Some(proto_block_id(block_number)),
        contract_address: Some(felt_to_proto(*contract_address)),
        key: Some(felt_to_proto(key)),
    }
    .encode_to_vec();

    pair(
        c,
        "parse_request/get_storage_at",
        &json,
        &grpc,
        |bytes| {
            let _: (ContractAddress, Felt, BlockIdOrTag) = serde_json::from_slice(bytes).unwrap();
        },
        |bytes| {
            let req = proto::GetStorageAtRequest::decode(bytes).unwrap();
            let _: BlockIdOrTag = req.block_id.unwrap().try_into().unwrap();
            let _: ContractAddress = req.contract_address.unwrap().try_into().unwrap();
            let _: Felt = req.key.unwrap().try_into().unwrap();
        },
    );
}

////////////////////////////////////////////////////////////////////////////////
// Response serialization: domain type -> wire bytes
////////////////////////////////////////////////////////////////////////////////

fn bench_serialize_invoke_response(c: &mut Criterion) {
    let response = AddInvokeTransactionResponse { transaction_hash: rand_felt(&mut make_rng()) };
    let json = serde_json::to_vec(&response).unwrap();
    let grpc = proto::AddInvokeTransactionResponse {
        transaction_hash: Some(felt_to_proto(response.transaction_hash)),
    }
    .encode_to_vec();

    pair(
        c,
        "serialize_response/invoke",
        &json,
        &grpc,
        |_| {
            let _ = black_box(serde_json::to_vec(&response).unwrap());
        },
        |_| {
            let _ = black_box(
                proto::AddInvokeTransactionResponse {
                    transaction_hash: Some(felt_to_proto(response.transaction_hash)),
                }
                .encode_to_vec(),
            );
        },
    );
}

fn bench_serialize_declare_response(c: &mut Criterion) {
    let mut rng = make_rng();
    let response = AddDeclareTransactionResponse {
        transaction_hash: rand_felt(&mut rng),
        class_hash: rand_felt(&mut rng),
    };
    let json = serde_json::to_vec(&response).unwrap();
    let grpc = proto::AddDeclareTransactionResponse {
        transaction_hash: Some(felt_to_proto(response.transaction_hash)),
        class_hash: Some(felt_to_proto(response.class_hash)),
    }
    .encode_to_vec();

    pair(
        c,
        "serialize_response/declare",
        &json,
        &grpc,
        |_| {
            let _ = black_box(serde_json::to_vec(&response).unwrap());
        },
        |_| {
            let _ = black_box(
                proto::AddDeclareTransactionResponse {
                    transaction_hash: Some(felt_to_proto(response.transaction_hash)),
                    class_hash: Some(felt_to_proto(response.class_hash)),
                }
                .encode_to_vec(),
            );
        },
    );
}

fn bench_serialize_deploy_account_response(c: &mut Criterion) {
    let mut rng = make_rng();
    let response = AddDeployAccountTransactionResponse {
        transaction_hash: rand_felt(&mut rng),
        contract_address: ContractAddress::from(rand_felt(&mut rng)),
    };
    let json = serde_json::to_vec(&response).unwrap();
    let grpc = proto::AddDeployAccountTransactionResponse {
        transaction_hash: Some(felt_to_proto(response.transaction_hash)),
        contract_address: Some(felt_to_proto(*response.contract_address)),
    }
    .encode_to_vec();

    pair(
        c,
        "serialize_response/deploy_account",
        &json,
        &grpc,
        |_| {
            let _ = black_box(serde_json::to_vec(&response).unwrap());
        },
        |_| {
            let _ = black_box(
                proto::AddDeployAccountTransactionResponse {
                    transaction_hash: Some(felt_to_proto(response.transaction_hash)),
                    contract_address: Some(felt_to_proto(*response.contract_address)),
                }
                .encode_to_vec(),
            );
        },
    );
}

fn bench_serialize_get_storage_at_response(c: &mut Criterion) {
    // `get_storage_at` returns a single felt (the storage value).
    let value = rand_felt(&mut make_rng());
    let json = serde_json::to_vec(&value).unwrap();
    let grpc = proto::GetStorageAtResponse { value: Some(felt_to_proto(value)) }.encode_to_vec();

    pair(
        c,
        "serialize_response/get_storage_at",
        &json,
        &grpc,
        |_| {
            let _ = black_box(serde_json::to_vec(&value).unwrap());
        },
        |_| {
            let _ = black_box(
                proto::GetStorageAtResponse { value: Some(felt_to_proto(value)) }.encode_to_vec(),
            );
        },
    );
}

fn bench_serialize_block_with_txs_response(c: &mut Criterion) {
    let block = block_response(&mut make_rng());
    let json = serde_json::to_vec(&block).unwrap();
    // Reuses the production `From<MaybePreConfirmedBlock>` impl (conversion/block.rs:139).
    let grpc = proto::GetBlockWithTxsResponse::from(block.clone()).encode_to_vec();

    pair(
        c,
        "serialize_response/get_block_with_txs",
        &json,
        &grpc,
        |_| {
            let _ = black_box(serde_json::to_vec(&block).unwrap());
        },
        |_| {
            let _ = black_box(proto::GetBlockWithTxsResponse::from(block.clone()).encode_to_vec());
        },
    );
}

fn bench_serialize_receipt_response(c: &mut Criterion) {
    let receipt = receipt_response(&mut make_rng());
    let json = serde_json::to_vec(&receipt).unwrap();
    // Reuses the production `From<&TxReceiptWithBlockInfo>` impl (conversion/receipt.rs:48).
    let grpc = proto::TransactionReceipt::from(&receipt).encode_to_vec();

    pair(
        c,
        "serialize_response/get_transaction_receipt",
        &json,
        &grpc,
        |_| {
            let _ = black_box(serde_json::to_vec(&receipt).unwrap());
        },
        |_| {
            let _ = black_box(proto::TransactionReceipt::from(&receipt).encode_to_vec());
        },
    );
}

criterion_group! {
    name = benches;
    config = Criterion::default().warm_up_time(Duration::from_millis(500));
    targets =
        bench_parse_invoke,
        bench_parse_declare,
        bench_parse_deploy_account,
        bench_parse_get_storage_at,
        bench_serialize_invoke_response,
        bench_serialize_declare_response,
        bench_serialize_deploy_account_response,
        bench_serialize_get_storage_at_response,
        bench_serialize_block_with_txs_response,
        bench_serialize_receipt_response,
}

criterion_main!(benches);
