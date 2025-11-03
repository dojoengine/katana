// Tests for the paymaster:
// - when starknet_estimateFee is called, for each tx of the batch the paymaster should:
//    + check if the sender_address is a controller acccount
//    + deploy the controller account if not already deployed.
// - when cartridge_addExecuteOutsideTransaction is called, the paymaster should deploy
//   the controller account if not already deployed.

use cartridge::rpc::types::{OutsideExecution, OutsideExecutionV2};
use katana_primitives::genesis::constant::DEFAULT_UDC_ADDRESS;

use assert_matches::assert_matches;
use katana_core::service::block_producer::BlockProducer;
use katana_executor::implementation::blockifier::BlockifierFactory;
use katana_pool::ordering::FiFo;
use katana_primitives::block::{BlockIdOrTag, BlockTag};
use katana_primitives::chain::ChainId;
use katana_primitives::fee::ResourceBoundsMapping;
use katana_rpc::starknet::{StarknetApi, StarknetApiConfig};
use katana_rpc_types::broadcasted::{BroadcastedInvokeTx, BroadcastedTx};
use katana_utils::node::{test_config, TestNode};
use serde_json::json;
use starknet::macros::selector;
use starknet::signers::SigningKey;
use std::sync::Arc;
use url::Url;

use cartridge::client::Client;
use cartridge::paymaster::{Error, Paymaster};
use katana_pool::TxPool;
use katana_primitives::contract::ContractAddress;
use katana_primitives::da::DataAvailabilityMode;
use katana_primitives::fee::{AllResourceBoundsMapping, ResourceBounds};
use katana_primitives::Felt;

const CONTROLLER_ADDRESS_1: &str = "0xb0b";
const CONTROLLER_ADDRESS_2: &str = "0xa11ce";

fn default_outside_execution() -> OutsideExecution {
    OutsideExecution::V2(OutsideExecutionV2 {
        caller: ContractAddress::from(Felt::ZERO),
        nonce: Felt::ZERO,
        execute_after: 0,
        execute_before: 0,
        calls: vec![],
    })
}

fn build_calldata(address: &str) -> Vec<&str> {
    vec![address, "0x42", address]
}

fn build_response(address: &str, is_controller: bool) -> String {
    if !is_controller {
        return "Address not found".to_string();
    }

    json!({
        "address": address,
        "username": "user",
        "calldata": build_calldata(address)
    })
    .to_string()
}

async fn build_mock(
    server: &mut mockito::ServerGuard,
    address: &str,
    is_controller: bool,
    expected_call_count: usize,
) -> mockito::Mock {
    let body = build_response(address, is_controller);
    server
        .mock("POST", "/accounts/calldata")
        .match_body(mockito::Matcher::Regex(address.to_string()))
        .with_body(body)
        .expect_at_least(expected_call_count)
        .create_async()
        .await
}

// spawn a mocked cartridge server
async fn setup_cartridge_server() -> mockito::ServerGuard {
    mockito::Server::new_async().await
}

// setup the mocks for the cartridge server
async fn setup_mocks(
    server: &mut mockito::ServerGuard,
    addresses: &[(&str, bool, bool)],
) -> Vec<mockito::Mock> {
    let mut mocks: Vec<mockito::Mock> = Vec::new();

    for (address, is_controller, must_be_called) in addresses {
        let expected_call_count = if *must_be_called { 1 } else { 0 };
        mocks.push(build_mock(server, address, *is_controller, expected_call_count).await);
    }
    mocks
}

/// Setup the test node and the paymaster.
async fn setup(
    cartridge_url: &str,
    paymaster_address: Option<ContractAddress>,
    paymaster_private_key: Option<SigningKey>,
) -> (TestNode, Paymaster<BlockifierFactory>) {
    let sequencer = TestNode::new().await;
    let block_producer = BlockProducer::instant(Arc::clone(&sequencer.backend()));
    let validator = block_producer.validator();

    let config = test_config();
    let starknet_api = StarknetApi::new(
        sequencer.backend().clone(),
        TxPool::new(validator.clone(), FiFo::new()),
        None,
        StarknetApiConfig {
            max_call_gas: config.rpc.max_call_gas,
            max_proof_keys: config.rpc.max_proof_keys,
            max_event_page_size: config.rpc.max_event_page_size,
            max_concurrent_estimate_fee_requests: config.rpc.max_concurrent_estimate_fee_requests,
        },
    );

    let client = Client::new(Url::parse(cartridge_url).unwrap());

    // use the first genesis account as the paymaster account
    let (account_address, account) = sequencer
        .backend()
        .chain_spec
        .genesis()
        .accounts()
        .nth(0)
        .expect("must have at least one account");

    let private_key = account.private_key().expect("must exist");

    let paymaster = Paymaster::new(
        starknet_api,
        client,
        TxPool::new(validator.clone(), FiFo::new()),
        ChainId::Id(Felt::ONE),
        paymaster_address.unwrap_or(*account_address),
        paymaster_private_key.unwrap_or(SigningKey::from_secret_scalar(private_key)),
    );

    (sequencer, paymaster)
}

/// Just build a fake invoke transaction.
fn invoke_tx(sender_address: ContractAddress) -> BroadcastedTx {
    BroadcastedTx::Invoke(BroadcastedInvokeTx {
        sender_address,
        calldata: vec![],
        signature: vec![],
        nonce: Felt::ZERO,
        paymaster_data: vec![],
        tip: 0,
        account_deployment_data: vec![],
        is_query: false,
        resource_bounds: ResourceBoundsMapping::All(AllResourceBoundsMapping {
            l1_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
            l1_data_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
            l2_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
        }),
        fee_data_availability_mode: DataAvailabilityMode::L1,
        nonce_data_availability_mode: DataAvailabilityMode::L1,
    })
}

fn assert_mocks(mocks: &[mockito::Mock]) {
    mocks.iter().for_each(|mock| mock.assert());
}

fn assert_tx(tx: &BroadcastedTx, address: &str) {
    if let BroadcastedTx::Invoke(tx) = tx {
        let calldata: Vec<Felt> =
            build_calldata(address).into_iter().map(|s| Felt::from_hex(&s).unwrap()).collect();
        assert_eq!(
            tx.calldata,
            vec![
                Felt::ONE,
                DEFAULT_UDC_ADDRESS.into(),
                selector!("deployContract"),
                calldata.len().into()
            ]
            .into_iter()
            .chain(calldata.into_iter())
            .collect::<Vec<Felt>>()
        );
    } else {
        panic!("expected invoke tx");
    };
}

#[tokio::test(flavor = "multi_thread")]
async fn test_starknet_estimate_fee_without_txs() {
    let server = setup_cartridge_server().await;
    let (_, paymaster) = setup(&server.url(), None, None).await;

    let res = paymaster.handle_estimate_fees(BlockIdOrTag::Tag(BlockTag::Pending), vec![]);

    assert!(res.is_ok());
    assert_eq!(res.unwrap().len(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_starknet_estimate_fee_when_sender_is_not_a_controller() {
    let mut server = setup_cartridge_server().await;
    let (_, paymaster) = setup(&server.url(), None, None).await;

    let mocks = setup_mocks(&mut server, &[(CONTROLLER_ADDRESS_1, false, true)]).await;

    let sender_address = ContractAddress::from(Felt::from_hex(CONTROLLER_ADDRESS_1).unwrap());
    let tx = invoke_tx(sender_address);

    let res = paymaster.handle_estimate_fees(BlockIdOrTag::Tag(BlockTag::Pending), vec![tx]);

    assert!(res.is_ok());
    assert_eq!(res.unwrap().len(), 0);
    assert_mocks(&mocks);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_starknet_estimate_fee_when_sender_is_an_already_deployed_controller() {
    let mut server = setup_cartridge_server().await;
    let (node, paymaster) = setup(&server.url(), None, None).await;

    let (sender_address, _) = node
        .backend()
        .chain_spec
        .genesis()
        .accounts()
        .nth(0)
        .expect("must have at least one account");

    let mocks =
        setup_mocks(&mut server, &[(sender_address.to_string().as_str(), true, false)]).await;

    let tx = invoke_tx(*sender_address);
    let res = paymaster.handle_estimate_fees(BlockIdOrTag::Tag(BlockTag::Pending), vec![tx]);

    assert!(res.is_ok());
    assert_eq!(res.unwrap().len(), 0);

    assert_mocks(&mocks);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_starknet_estimate_fee_when_sender_is_a_not_yet_deployed_controller() {
    let mut server = setup_cartridge_server().await;
    let (_, paymaster) = setup(&server.url(), None, None).await;

    let mocks = setup_mocks(&mut server, &[(CONTROLLER_ADDRESS_1, true, true)]).await;

    let sender_address = ContractAddress::from(Felt::from_hex(CONTROLLER_ADDRESS_1).unwrap());
    let tx = invoke_tx(sender_address);

    let res = paymaster.handle_estimate_fees(BlockIdOrTag::Tag(BlockTag::Pending), vec![tx]);

    assert!(res.is_ok());
    let res = res.unwrap();
    assert_eq!(res.len(), 1);
    assert_tx(&res[0], CONTROLLER_ADDRESS_1);

    assert_mocks(&mocks);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_starknet_estimate_fee_when_several_txs_with_the_same_not_yet_deployed_controller() {
    let mut server = setup_cartridge_server().await;
    let (_, paymaster) = setup(&server.url(), None, None).await;

    let mocks = setup_mocks(&mut server, &[(CONTROLLER_ADDRESS_1, true, true)]).await;

    let sender_address = ContractAddress::from(Felt::from_hex(CONTROLLER_ADDRESS_1).unwrap());
    let txs = vec![invoke_tx(sender_address), invoke_tx(sender_address)];

    let res = paymaster.handle_estimate_fees(BlockIdOrTag::Tag(BlockTag::Pending), txs);

    assert!(res.is_ok());

    let res = res.unwrap();
    assert_eq!(res.len(), 1);
    assert_tx(&res[0], CONTROLLER_ADDRESS_1);
    assert_mocks(&mocks);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_starknet_estimate_fee_when_several_txs_with_several_controllers() {
    let mut server = setup_cartridge_server().await;
    let (_, paymaster) = setup(&server.url(), None, None).await;

    let mocks = setup_mocks(
        &mut server,
        &[(CONTROLLER_ADDRESS_1, true, true), (CONTROLLER_ADDRESS_2, true, true)],
    )
    .await;

    let txs = vec![
        invoke_tx(ContractAddress::from(Felt::from_hex(CONTROLLER_ADDRESS_1).unwrap())),
        invoke_tx(ContractAddress::from(Felt::from_hex(CONTROLLER_ADDRESS_2).unwrap())),
    ];

    let res = paymaster.handle_estimate_fees(BlockIdOrTag::Tag(BlockTag::Pending), txs);

    assert!(res.is_ok());

    let res = res.unwrap();
    assert_eq!(res.len(), 2);
    assert_tx(&res[0], CONTROLLER_ADDRESS_1);
    assert_tx(&res[1], CONTROLLER_ADDRESS_2);

    assert_mocks(&mocks);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_starknet_estimate_fee_sender_with_invalid_paymaster_config() {
    let mut server = setup_cartridge_server().await;

    // configure a wrong paymaster address
    let wrong_paymaster_address = ContractAddress::from(Felt::THREE);
    let (_, paymaster) = setup(&server.url(), Some(wrong_paymaster_address), None).await;

    let mocks = setup_mocks(&mut server, &[(CONTROLLER_ADDRESS_1, true, true)]).await;

    let sender_address = ContractAddress::from(Felt::from_hex(CONTROLLER_ADDRESS_1).unwrap());
    let tx = invoke_tx(sender_address);

    let res = paymaster.handle_estimate_fees(BlockIdOrTag::Tag(BlockTag::Pending), vec![tx]);

    assert!(res.is_err());
    assert_matches!(res.unwrap_err(), Error::PaymasterNotFound(_));
    assert_mocks(&mocks);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_starknet_estimate_fee_with_a_mix_of_txs() {
    let mut server = setup_cartridge_server().await;
    let (_, paymaster) = setup(&server.url(), None, None).await;

    let mocks = setup_mocks(
        &mut server,
        &[
            ("0x12345", false, false),
            ("0x67890", false, false),
            (CONTROLLER_ADDRESS_1, true, true),
            (CONTROLLER_ADDRESS_2, true, true),
        ],
    )
    .await;

    let txs = vec![
        invoke_tx(ContractAddress::from(Felt::from_hex("0x67890").unwrap())),
        invoke_tx(ContractAddress::from(Felt::from_hex(CONTROLLER_ADDRESS_2).unwrap())),
        invoke_tx(ContractAddress::from(Felt::from_hex("0x12345").unwrap())),
        invoke_tx(ContractAddress::from(Felt::from_hex("0x67890").unwrap())),
        invoke_tx(ContractAddress::from(Felt::from_hex(CONTROLLER_ADDRESS_1).unwrap())),
        invoke_tx(ContractAddress::from(Felt::from_hex(CONTROLLER_ADDRESS_2).unwrap())),
    ];

    let res = paymaster.handle_estimate_fees(BlockIdOrTag::Tag(BlockTag::Pending), txs);

    assert!(res.is_ok());

    let res = res.unwrap();
    assert_eq!(res.len(), 2);
    assert_tx(&res[0], CONTROLLER_ADDRESS_2);
    assert_tx(&res[1], CONTROLLER_ADDRESS_1);

    assert_mocks(&mocks);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_cartridge_outside_transaction_when_caller_is_not_a_controller() {
    let mut server = setup_cartridge_server().await;
    let (_, paymaster) = setup(&server.url(), None, None).await;

    let mocks = setup_mocks(&mut server, &[(CONTROLLER_ADDRESS_1, false, false)]).await;

    let res = paymaster.handle_add_outside_execution(
        ContractAddress::from(Felt::from_hex(CONTROLLER_ADDRESS_1).unwrap()),
        default_outside_execution(),
        vec![],
    );

    assert!(res.is_ok());
    assert!(res.unwrap().is_none());

    assert_mocks(&mocks);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_cartridge_outside_transaction_when_caller_is_an_already_deployed_controller() {
    let mut server = setup_cartridge_server().await;
    let (node, paymaster) = setup(&server.url(), None, None).await;

    let (caller_address, _) = node
        .backend()
        .chain_spec
        .genesis()
        .accounts()
        .nth(0)
        .expect("must have at least one account");

    let mocks =
        setup_mocks(&mut server, &[(caller_address.to_string().as_str(), true, false)]).await;

    let res = paymaster.handle_add_outside_execution(
        *caller_address,
        default_outside_execution(),
        vec![],
    );

    assert!(res.is_ok());
    assert!(res.unwrap().is_none());

    assert_mocks(&mocks);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_cartridge_outside_transaction_when_caller_is_a_not_yet_deployed_controller() {
    let mut server = setup_cartridge_server().await;
    let (_, paymaster) = setup(&server.url(), None, None).await;

    let mocks = setup_mocks(&mut server, &[(CONTROLLER_ADDRESS_2, true, true)]).await;

    let res = paymaster.handle_add_outside_execution(
        ContractAddress::from(Felt::from_hex(CONTROLLER_ADDRESS_2).unwrap()),
        default_outside_execution(),
        vec![],
    );

    assert!(res.is_ok());
    assert!(res.unwrap().is_some());

    assert_mocks(&mocks);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_cartridge_outside_transaction_when_caller_is_a_not_yet_deployed_but_wrong_paymaster_pkey(
) {
    let mut server = setup_cartridge_server().await;
    let (_, paymaster) =
        setup(&server.url(), None, Some(SigningKey::from_secret_scalar(Felt::THREE))).await;

    let mocks = setup_mocks(&mut server, &[(CONTROLLER_ADDRESS_2, true, true)]).await;

    let res = paymaster.handle_add_outside_execution(
        ContractAddress::from(Felt::from_hex(CONTROLLER_ADDRESS_2).unwrap()),
        default_outside_execution(),
        vec![],
    );

    assert!(res.is_err());
    assert_matches!(res.unwrap_err(), Error::FailedToAddTransaction(_));
    assert_mocks(&mocks);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_cartridge_outside_transaction_when_paymaster_is_not_properly_configured() {
    let mut server = setup_cartridge_server().await;
    let (_, paymaster) = setup(&server.url(), Some(ContractAddress::from(Felt::THREE)), None).await;

    let mocks = setup_mocks(&mut server, &[(CONTROLLER_ADDRESS_1, true, true)]).await;

    let res = paymaster.handle_add_outside_execution(
        ContractAddress::from(Felt::from_hex(CONTROLLER_ADDRESS_1).unwrap()),
        default_outside_execution(),
        vec![],
    );

    assert!(res.is_err());
    assert_matches!(res.unwrap_err(), Error::PaymasterNotFound(_));
    assert_mocks(&mocks);
}
