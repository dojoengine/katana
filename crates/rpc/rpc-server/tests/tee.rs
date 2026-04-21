#![cfg(feature = "tee-mock")]

//! Tests for `TeeApi::generate_quote`.
//!
//! `TeeApi<DbProviderFactory>` is constructed directly against an in-memory provider —
//! no HTTP server needed because the method is a thin wrapper over provider reads plus
//! a TEE provider call. `MockProvider` lays out its output as
//! `"MOCK" (4 bytes) | user_data (64 bytes) | checksum (4 bytes)`, so decoding the
//! response's `quote` hex and slicing `[4..68]` recovers the exact `report_data` the
//! API passed to the TEE provider — giving us a cryptographic handle on whether the
//! response is bound to the requested inputs.

use std::sync::Arc;

use assert_matches::assert_matches;
use katana_primitives::block::{Block, BlockNumber, FinalityStatus, Header, SealedBlockWithStatus};
use katana_primitives::fee::FeeInfo;
use katana_primitives::hash::{Poseidon, StarkHash};
use katana_primitives::receipt::{
    ExecutionResources, InvokeTxReceipt, L1HandlerTxReceipt, MessageToL1, Receipt,
};
use katana_primitives::transaction::{InvokeTx, InvokeTxV1, L1HandlerTx, Tx, TxWithHash};
use katana_primitives::{address, felt, ContractAddress, Felt, B256};
use katana_provider::api::block::BlockWriter;
use katana_provider::{DbProviderFactory, MutableProvider, ProviderFactory};
use katana_rpc_api::tee::{TeeApiServer, TeeL1ToL2Message, TeeL2ToL1Message};
use katana_rpc_server::tee::TeeApi;
use katana_tee::MockProvider;

// ---------- helpers ----------

fn mock_api(
    factory: DbProviderFactory,
    fork_block_number: Option<u64>,
) -> TeeApi<DbProviderFactory> {
    TeeApi::new(factory, Arc::new(MockProvider::new()), fork_block_number)
}

fn make_block(
    number: BlockNumber,
    hash: Felt,
    state_root: Felt,
    events_commitment: Felt,
    body: Vec<TxWithHash>,
) -> SealedBlockWithStatus {
    let header = Header { number, state_root, events_commitment, ..Default::default() };
    Block { header, body }.seal_with_hash_and_status(hash, FinalityStatus::AcceptedOnL2)
}

fn insert(factory: &DbProviderFactory, block: SealedBlockWithStatus, receipts: Vec<Receipt>) {
    let pm = factory.provider_mut();
    pm.insert_block_with_states_and_receipts(block, Default::default(), receipts, Vec::new())
        .expect("insert block");
    pm.commit().expect("commit");
}

/// Extract the 64-byte `report_data` that was passed to the mock TEE provider.
///
/// See [`katana_tee::MockProvider`] — layout: `"MOCK" | user_data | checksum_le_u32`.
fn extract_report_data(quote_hex: &str) -> [u8; 64] {
    let bytes =
        hex::decode(quote_hex.strip_prefix("0x").expect("quote has 0x prefix")).expect("valid hex");
    assert_eq!(&bytes[0..4], b"MOCK", "mock magic header");
    assert_eq!(bytes.len(), 4 + 64 + 4, "mock quote length");
    let mut out = [0u8; 64];
    out.copy_from_slice(&bytes[4..68]);
    out
}

fn appchain_report_data(fields: [Felt; 7]) -> [u8; 64] {
    let mut out = [0u8; 64];
    out[..32].copy_from_slice(&Poseidon::hash_array(&fields).to_bytes_be());
    out
}

fn sharding_report_data(fields: [Felt; 8]) -> [u8; 64] {
    let mut out = [0u8; 64];
    out[..32].copy_from_slice(&Poseidon::hash_array(&fields).to_bytes_be());
    out
}

fn empty_messages_commitment() -> Felt {
    Poseidon::hash_array(&[Poseidon::hash_array(&[]), Poseidon::hash_array(&[])])
}

fn l2_to_l1_hash(msg: &MessageToL1) -> Felt {
    let len = Felt::from(msg.payload.len());
    let payload_hash = Poseidon::hash_array(
        &std::iter::once(len).chain(msg.payload.iter().copied()).collect::<Vec<_>>(),
    );
    Poseidon::hash_array(&[msg.from_address.into(), msg.to_address, payload_hash])
}

fn invoke_receipt(messages_sent: Vec<MessageToL1>) -> Receipt {
    Receipt::Invoke(InvokeTxReceipt {
        fee: FeeInfo::default(),
        events: Vec::new(),
        messages_sent,
        revert_error: None,
        execution_resources: ExecutionResources::default(),
    })
}

fn l1_handler_receipt(message_hash: B256) -> Receipt {
    Receipt::L1Handler(L1HandlerTxReceipt {
        fee: FeeInfo::default(),
        events: Vec::new(),
        message_hash,
        messages_sent: Vec::new(),
        revert_error: None,
        execution_resources: ExecutionResources::default(),
    })
}

fn invoke_tx(hash: Felt) -> TxWithHash {
    TxWithHash { hash, transaction: Tx::Invoke(InvokeTx::V1(InvokeTxV1::default())) }
}

fn l1_handler_tx(
    hash: Felt,
    nonce: Felt,
    contract_address: ContractAddress,
    selector: Felt,
    calldata: Vec<Felt>,
) -> TxWithHash {
    TxWithHash {
        hash,
        transaction: Tx::L1Handler(L1HandlerTx {
            nonce,
            calldata,
            contract_address,
            entry_point_selector: selector,
            ..Default::default()
        }),
    }
}

// ---------- tests ----------

/// Genesis path: `prev_block = None` folds to the `(Felt::MAX, ZERO, ZERO)` sentinel
/// and the appchain `report_data` formula is used. Asserts the full response shape and
/// that the mock quote's embedded `report_data` matches the formula applied to the
/// inserted block's hash/roots.
#[tokio::test]
async fn generate_quote_genesis_appchain() {
    let factory = DbProviderFactory::new_in_memory();

    let block_hash = felt!("0xb10c");
    let state_root = felt!("0x5747");
    let events_commitment = felt!("0xe0");

    insert(
        &factory,
        make_block(0, block_hash, state_root, events_commitment, Vec::new()),
        Vec::new(),
    );

    let api = mock_api(factory, None);
    let resp = api.generate_quote(None, 0).await.expect("generate_quote");

    assert_eq!(resp.prev_block_number, None);
    assert_eq!(resp.block_number, 0);
    assert_eq!(resp.fork_block_number, None);
    assert_eq!(resp.prev_state_root, Felt::ZERO);
    assert_eq!(resp.prev_block_hash, Felt::ZERO);
    assert_eq!(resp.state_root, state_root);
    assert_eq!(resp.block_hash, block_hash);
    assert_eq!(resp.events_commitment, events_commitment);
    assert!(resp.l1_to_l2_messages.is_empty());
    assert!(resp.l2_to_l1_messages.is_empty());
    assert_eq!(resp.messages_commitment, empty_messages_commitment());

    let expected_report_data = appchain_report_data([
        Felt::ZERO, // prev_state_root
        state_root,
        Felt::ZERO, // prev_block_hash
        block_hash,
        Felt::MAX,  // prev_block_id (genesis sentinel)
        Felt::ZERO, // block
        empty_messages_commitment(),
    ]);
    assert_eq!(extract_report_data(&resp.quote), expected_report_data);
}

/// Fork / sharding path: `fork_block_number = Some` selects the 8-field `report_data`
/// that includes `events_commitment + fork_block_number`, *and* short-circuits message
/// aggregation — even though block 1's receipt carries a `messages_sent` entry, the
/// response has empty message vectors and `messages_commitment = Felt::ZERO`.
#[tokio::test]
async fn generate_quote_fork_sharding_mode() {
    let factory = DbProviderFactory::new_in_memory();

    // block 0 — referenced as prev_block so we exercise the Some-prev branch too.
    let h0 = felt!("0xa0");
    let sr0 = felt!("0xa1");
    insert(&factory, make_block(0, h0, sr0, felt!("0xe0"), Vec::new()), Vec::new());

    // block 1 has a message that MUST be suppressed by the fork path.
    let h1 = felt!("0xb0");
    let sr1 = felt!("0xb1");
    let ec1 = felt!("0xe1");
    let ignored_msg = MessageToL1 {
        from_address: address!("0xdead"),
        to_address: felt!("0xbeef"),
        payload: vec![felt!("0x1")],
    };
    insert(
        &factory,
        make_block(1, h1, sr1, ec1, vec![invoke_tx(felt!("0xf00"))]),
        vec![invoke_receipt(vec![ignored_msg])],
    );

    let fork_block = 42u64;
    let api = mock_api(factory, Some(fork_block));
    let resp = api.generate_quote(Some(0), 1).await.expect("generate_quote");

    assert_eq!(resp.prev_block_number, Some(0));
    assert_eq!(resp.block_number, 1);
    assert_eq!(resp.fork_block_number, Some(fork_block));
    assert_eq!(resp.prev_block_hash, h0);
    assert_eq!(resp.prev_state_root, sr0);
    assert_eq!(resp.block_hash, h1);
    assert_eq!(resp.state_root, sr1);
    assert_eq!(resp.events_commitment, ec1);

    // Fork mode MUST NOT aggregate messages.
    assert!(resp.l1_to_l2_messages.is_empty());
    assert!(resp.l2_to_l1_messages.is_empty());
    assert_eq!(resp.messages_commitment, Felt::ZERO);

    let expected_report_data = sharding_report_data([
        sr0,
        sr1,
        h0,
        h1,
        Felt::ZERO, // prev_block_id = Felt::from(0)
        Felt::ONE,  // block = 1
        Felt::from(fork_block),
        ec1,
    ]);
    assert_eq!(extract_report_data(&resp.quote), expected_report_data);
}

/// L2→L1 aggregation across a multi-block range (`prev_block + 1 ..= block`).
/// Also covers the `Some(prev_block)` provider-fetch path for the prev block's hash/root.
/// Validates the exact hash formula: `Poseidon([from, to, Poseidon([len, ...payload])])`.
#[tokio::test]
async fn generate_quote_l2_to_l1_message_aggregation() {
    let factory = DbProviderFactory::new_in_memory();

    // Block 0 — only referenced as the prev_block; contributes no messages.
    let h0 = felt!("0xa0");
    let sr0 = felt!("0xa1");
    insert(&factory, make_block(0, h0, sr0, Felt::ZERO, Vec::new()), Vec::new());

    // Block 1: one receipt with one L2→L1 message.
    let msg_a = MessageToL1 {
        from_address: address!("0x111"),
        to_address: felt!("0x222"),
        payload: vec![felt!("0x1"), felt!("0x2")],
    };
    let h1 = felt!("0xb0");
    let sr1 = felt!("0xb1");
    insert(
        &factory,
        make_block(1, h1, sr1, Felt::ZERO, vec![invoke_tx(felt!("0xf01"))]),
        vec![invoke_receipt(vec![msg_a.clone()])],
    );

    // Block 2: one receipt with two L2→L1 messages — exercises intra-receipt ordering.
    let msg_b = MessageToL1 {
        from_address: address!("0x333"),
        to_address: felt!("0x444"),
        payload: Vec::new(), // empty payload is a valid edge
    };
    let msg_c = MessageToL1 {
        from_address: address!("0x555"),
        to_address: felt!("0x666"),
        payload: vec![felt!("0x9")],
    };
    let h2 = felt!("0xc0");
    let sr2 = felt!("0xc1");
    let ec2 = felt!("0xc2");
    insert(
        &factory,
        make_block(2, h2, sr2, ec2, vec![invoke_tx(felt!("0xf02"))]),
        vec![invoke_receipt(vec![msg_b.clone(), msg_c.clone()])],
    );

    let api = mock_api(factory, None);
    let resp = api.generate_quote(Some(0), 2).await.expect("generate_quote");

    let expected_msgs = [&msg_a, &msg_b, &msg_c].map(|m| TeeL2ToL1Message {
        from_address: m.from_address.into(),
        to_address: m.to_address,
        payload: m.payload.clone(),
    });
    assert_eq!(resp.l2_to_l1_messages, expected_msgs.to_vec());
    assert!(resp.l1_to_l2_messages.is_empty());

    let hashes: Vec<Felt> = [&msg_a, &msg_b, &msg_c].iter().map(|m| l2_to_l1_hash(m)).collect();
    let expected_commitment =
        Poseidon::hash_array(&[Poseidon::hash_array(&hashes), Poseidon::hash_array(&[])]);
    assert_eq!(resp.messages_commitment, expected_commitment);

    assert_eq!(resp.prev_block_hash, h0);
    assert_eq!(resp.prev_state_root, sr0);
    assert_eq!(resp.block_hash, h2);
    assert_eq!(resp.state_root, sr2);
    assert_eq!(resp.events_commitment, ec2);

    let expected_report_data = appchain_report_data([
        sr0,
        sr2,
        h0,
        h2,
        Felt::ZERO, // prev_block_id = Felt::from(0)
        Felt::from(2u64),
        expected_commitment,
    ]);
    assert_eq!(extract_report_data(&resp.quote), expected_report_data);
}

/// L1→L2 aggregation reads from two distinct sources in the same block:
/// - `message_hash` from `Receipt::L1Handler` feeds the commitment.
/// - `calldata` / `contract_address` / `selector` / `nonce` from `Tx::L1Handler` feed the
///   response's `l1_to_l2_messages` field.
/// This test exercises both and validates they line up for the same block.
#[tokio::test]
async fn generate_quote_l1_to_l2_message_aggregation() {
    let factory = DbProviderFactory::new_in_memory();

    // Block 0: contains the L1Handler tx + matching receipt.
    let h0 = felt!("0xa0");
    let sr0 = felt!("0xa1");

    // Pick a message_hash that fits cleanly into a Felt (top bits zero).
    let message_hash = B256::with_last_byte(0x42);
    let sender_on_l1 = felt!("0xe7"); // calldata[0]
    let payload_elems = vec![felt!("0x1"), felt!("0x2"), felt!("0x3")];
    let mut calldata = vec![sender_on_l1];
    calldata.extend(payload_elems.iter().copied());

    let contract_address = address!("0xdeadbeef");
    let selector = felt!("0x5e1");
    let nonce = felt!("0x7");

    let tx = l1_handler_tx(felt!("0xf01"), nonce, contract_address, selector, calldata);
    let receipt = l1_handler_receipt(message_hash);

    insert(&factory, make_block(0, h0, sr0, Felt::ZERO, vec![tx]), vec![receipt]);

    let api = mock_api(factory, None);
    let resp = api.generate_quote(None, 0).await.expect("generate_quote");

    assert!(resp.l2_to_l1_messages.is_empty());
    assert_eq!(
        resp.l1_to_l2_messages,
        vec![TeeL1ToL2Message {
            from_address: sender_on_l1,
            to_address: contract_address.into(),
            selector,
            payload: payload_elems,
            nonce,
        }]
    );

    // L1→L2 hash fed into the commitment comes from the *receipt's* message_hash.
    let l1_to_l2_hash = Felt::from_bytes_be_slice(&message_hash.0);
    let expected_commitment = Poseidon::hash_array(&[
        Poseidon::hash_array(&[]), // no L2→L1
        Poseidon::hash_array(&[l1_to_l2_hash]),
    ]);
    assert_eq!(resp.messages_commitment, expected_commitment);

    // Verify the same commitment was bound into the report_data fed to the TEE provider —
    // guards against a bug where the response field and the attested value diverge.
    let expected_report_data = appchain_report_data([
        Felt::ZERO, // prev_state_root
        sr0,
        Felt::ZERO, // prev_block_hash
        h0,
        Felt::MAX,  // prev_block_id (genesis sentinel)
        Felt::ZERO, // block
        expected_commitment,
    ]);
    assert_eq!(extract_report_data(&resp.quote), expected_report_data);
}

/// Missing `prev_block` surfaces as `TeeApiError::ProviderError` (code 102), not a panic
/// or a silent zero. Distinguishes this error from other RPC errors by its custom code.
#[tokio::test]
async fn generate_quote_missing_prev_block_errors() {
    let factory = DbProviderFactory::new_in_memory();
    insert(&factory, make_block(0, felt!("0xa0"), Felt::ZERO, Felt::ZERO, Vec::new()), Vec::new());

    let api = mock_api(factory, None);
    // prev_block = 5 doesn't exist; block = 0 does.
    let err = api.generate_quote(Some(5), 0).await.expect_err("expected ProviderError");

    assert_eq!(err.code(), 102, "TeeApiError::ProviderError code");
    assert!(
        err.message().contains("Block hash not found for block 5"),
        "error should name the missing block: {}",
        err.message()
    );
}

/// Wire-format contract for `prev_block_number`: custom serde maps `None ↔ Felt::MAX`
/// (not `None ↔ null`) so the JSON is compatible with `katana_tee_client::TeeQuoteResponse`
/// on the saya-tee side. Validates both branches *and* that round-trip restores the
/// original `Option<BlockNumber>` value.
#[tokio::test]
async fn generate_quote_prev_block_number_wire_format() {
    let factory = DbProviderFactory::new_in_memory();
    insert(&factory, make_block(0, felt!("0xa0"), Felt::ZERO, Felt::ZERO, Vec::new()), Vec::new());
    insert(&factory, make_block(1, felt!("0xb0"), Felt::ZERO, Felt::ZERO, Vec::new()), Vec::new());

    let api = mock_api(factory, None);

    // Case 1: None → serialized as Felt::MAX.
    let resp_none = api.generate_quote(None, 0).await.expect("None case");
    let json_none = serde_json::to_value(&resp_none).expect("serialize");
    assert_eq!(
        json_none["prevBlockNumber"],
        serde_json::to_value(Felt::MAX).unwrap(),
        "None must serialize to Felt::MAX, not null"
    );
    assert!(
        !json_none["prevBlockNumber"].is_null(),
        "None must NOT serialize to JSON null (breaks saya-tee compatibility)"
    );
    let round_trip_none: katana_rpc_api::tee::TeeQuoteResponse =
        serde_json::from_value(json_none).expect("deserialize");
    assert_matches!(round_trip_none.prev_block_number, None);

    // Case 2: Some(0) → serialized as Felt::from(0). Using Some(0) specifically exercises
    // the boundary where the value is not the Felt::MAX sentinel but is also numerically zero.
    let resp_some = api.generate_quote(Some(0), 1).await.expect("Some case");
    let json_some = serde_json::to_value(&resp_some).expect("serialize");
    assert_eq!(
        json_some["prevBlockNumber"],
        serde_json::to_value(Felt::from(0u64)).unwrap(),
        "Some(0) must serialize to the Felt form of 0"
    );
    let round_trip_some: katana_rpc_api::tee::TeeQuoteResponse =
        serde_json::from_value(json_some).expect("deserialize");
    assert_eq!(round_trip_some.prev_block_number, Some(0));
}
