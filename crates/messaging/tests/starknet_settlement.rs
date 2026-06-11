//! Integration tests for the messaging service against a Starknet settlement layer.
//!
//! A second Katana node (via `TestNode`) is used purely as the *settlement*
//! chain — it runs Starknet RPC and hosts the `piltover_messaging_mock`
//! contract whose `send_message_to_appchain` emits the `MessageSent` event the
//! `StarknetCollector` matches on. The L2 side is just the isolated
//! `MessagingService` with an in-memory pool and provider.

use std::path::PathBuf;
use std::time::Duration;

use katana_messaging::{MessagingService, SettlementChainConfig};
use katana_pool::api::TransactionPool;
use katana_primitives::chain::ChainId;
use katana_primitives::transaction::ExecutableTx;
use katana_primitives::{ContractAddress, Felt};
use katana_utils::{TestNode, TxWaiter};
use starknet::accounts::Account;
use starknet::contract::{ContractFactory, UdcSelector};
use starknet::core::types::Call;
use starknet::macros::selector;
use url::Url;

mod common;

/// Path (relative to the workspace root, since `cargo test` runs with CWD =
/// crate root) to the precompiled piltover messaging mock. This contract
/// exposes `send_message_to_appchain` and emits the `MessageSent` event the
/// `StarknetCollector` filters on.
const MESSAGING_MOCK_ARTIFACT: &str =
    "../contracts/build/piltover_messaging_mock.contract_class.json";

#[tokio::test(flavor = "multi_thread")]
async fn collects_single_message_from_starknet_settlement() {
    // The settlement chain: a real Katana running the messaging mock.
    // Bump the RPC's `max_event_page_size` past the collector's chunk size
    // (200) so the collector's paged `get_events` calls aren't rejected.
    let mut config = katana_utils::node::test_config();
    config.rpc.max_event_page_size = Some(1024);
    let settlement_node = TestNode::new_with_config(config).await;
    let settlement_account = settlement_node.account();
    let rpc_client = settlement_node.starknet_rpc_client();

    // Declare the messaging mock on the settlement chain.
    let artifact = PathBuf::from(MESSAGING_MOCK_ARTIFACT);
    let (contract, compiled_hash) =
        common::prepare_contract_declaration_params(&artifact).expect("read mock artifact");
    let class_hash = contract.class_hash();
    let declare = settlement_account
        .declare_v3(contract.into(), compiled_hash)
        .send()
        .await
        .expect("declare messaging mock");
    TxWaiter::new(declare.transaction_hash, &rpc_client).await.expect("declare tx failed");

    // Deploy the mock with cancellation_delay_secs=0.
    let factory = ContractFactory::new_with_udc(class_hash, &settlement_account, UdcSelector::New);
    let deployment = factory.deploy_v3(vec![Felt::ZERO], Felt::ZERO, false);
    let messaging_contract: ContractAddress = deployment.deployed_address().into();
    let deploy = deployment.send().await.expect("deploy messaging mock");
    TxWaiter::new(deploy.transaction_hash, &rpc_client).await.expect("deploy tx failed");

    // Wire up the isolated messaging service against the settlement RPC.
    let rpc_url =
        Url::parse(&format!("http://{}", settlement_node.rpc_addr())).expect("settlement rpc url");
    let settlement =
        SettlementChainConfig::Starknet { rpc_url, contract_address: messaging_contract };

    let pool = common::build_test_pool();
    let provider = common::build_test_provider();

    let service =
        MessagingService::new(ChainId::default(), pool.clone(), provider.clone(), settlement)
            .interval(1)
            .from_block(0);

    let mut handle = service.start().expect("start messaging service");

    // Fire `send_message_to_appchain(to, selector, payload)` on the settlement
    // chain. The mock emits `MessageSent` which the collector picks up.
    let recipient = ContractAddress::from(Felt::from(0xbeef_u64));
    let entry_point_selector = selector!("msg_handler_value");
    let payload = vec![Felt::from(123_u64)];

    let mut calldata =
        vec![recipient.into(), entry_point_selector, Felt::from(payload.len() as u64)];
    calldata.extend_from_slice(&payload);

    let call = Call {
        to: messaging_contract.into(),
        selector: selector!("send_message_to_appchain"),
        calldata,
    };

    let invoke = settlement_account
        .execute_v3(vec![call])
        .send()
        .await
        .expect("invoke send_message_to_appchain");
    TxWaiter::new(invoke.transaction_hash, &rpc_client).await.expect("invoke tx failed");

    // Wait for the messaging service to pick the event up.
    common::wait_for_pool_size(&pool, 1, Duration::from_secs(15))
        .await
        .expect("messaging service should have collected the message");

    let snapshot = pool.take_transactions_snapshot();
    assert_eq!(snapshot.len(), 1);
    let ExecutableTx::L1Handler(ref l1_handler) = snapshot[0].transaction else {
        panic!("expected L1Handler tx in pool");
    };

    assert_eq!(l1_handler.contract_address, recipient);
    assert_eq!(l1_handler.entry_point_selector, entry_point_selector);
    // Calldata = [from_address (sender on settlement), ...payload]
    let sender_felt: Felt = settlement_account.address();
    assert_eq!(l1_handler.calldata[0], sender_felt);
    assert_eq!(l1_handler.calldata[1..], payload[..]);

    // The L1->L2 index records the settlement-chain tx hash.
    let settlement_tx_hash_bytes = invoke.transaction_hash.to_bytes_be();
    let l2_hashes = common::l2_txs_for_l1(&provider, &settlement_tx_hash_bytes);
    assert_eq!(l2_hashes, vec![snapshot[0].hash]);

    // Checkpoint advanced past the invoke's block.
    let cp = common::messaging_checkpoint(&provider).expect("checkpoint persisted");
    let invoke_block = rpc_client
        .get_transaction_receipt(invoke.transaction_hash)
        .await
        .expect("get receipt")
        .block
        .block_number();
    assert!(
        cp.block >= invoke_block,
        "checkpoint block {} should be >= invoke block {}",
        cp.block,
        invoke_block,
    );

    handle.stop();
    handle.stopped().await;
}
