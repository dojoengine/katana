use std::path::PathBuf;

use katana_test_utils::prepare_contract_declaration_params;
use starknet::core::utils::get_selector_from_name;
use starknet_crypto::Felt;

use std::sync::Arc;

use crate::client::Client;
use crate::paymaster::{Error, Paymaster};
use crate::rpc::types::{
    Call, NonceChannel, OutsideExecution, OutsideExecutionV2, OutsideExecutionV3,
};
use crate::vrf::{VrfContext, CARTRIDGE_VRF_CLASS_HASH, CARTRIDGE_VRF_SALT};
use assert_matches::assert_matches;
use futures::StreamExt;
use katana_core::service::block_producer::BlockProducer;
use katana_executor::implementation::blockifier::BlockifierFactory;
use katana_pool::ordering::FiFo;
use katana_pool::{TransactionPool, TxPool};
use katana_primitives::address;
use katana_primitives::block::{BlockIdOrTag, BlockTag};
use katana_primitives::chain::ChainId;
use katana_primitives::contract::ContractAddress;
use katana_primitives::da::DataAvailabilityMode;
use katana_primitives::fee::{AllResourceBoundsMapping, ResourceBounds, ResourceBoundsMapping};
use katana_primitives::genesis::constant::DEFAULT_UDC_ADDRESS;
use katana_primitives::transaction::{ExecutableTx, InvokeTx};
use katana_rpc::starknet::{StarknetApi, StarknetApiConfig};
use katana_rpc_types::broadcasted::{BroadcastedInvokeTx, BroadcastedTx};
use katana_utils::node::{test_config, TestNode};
use katana_utils::TxWaiter;
use serde_json::json;
use starknet::accounts::Account;
use starknet::core::types::Call as StarknetCall;
use starknet::macros::{felt, selector};
use starknet::signers::SigningKey;
use url::Url;

const DEFAULT_NONCE_CHANNEL: u128 = 10;

const CONTROLLER_ADDRESS_1: Felt = felt!("0xb0b");
const CONTROLLER_ADDRESS_2: Felt = felt!("0xa11ce");

const ALREADY_DEPLOYED_CONTROLLER_ADDRESS: Felt =
    felt!("0x1399f8d212a38c4f3b3080573660a15bd26d15036b7638b63540aaefe135c49");

const VRF_PROVIDER_CLASS_PATH: &str =
    "src/paymaster/tests/test_data/cartridge_vrf_VrfProvider.contract_class.json";
const VRF_PRIVATE_KEY_FOR_TESTS: Felt = felt!("0xdeadbeef");
const VRF_PUBLIC_KEY_X: Felt =
    felt!("0x57641624f71ce549c59b6d7245c9df254f7a2b183c296d0a64fcee941e753f7");
const VRF_PUBLIC_KEY_Y: Felt =
    felt!("0x24d0c384cc7471e2a68e7d8085f8c25a171863eeb2b6e433da036e287932fe");

fn default_outside_execution() -> OutsideExecution {
    OutsideExecution::V2(OutsideExecutionV2 {
        caller: ContractAddress::from(Felt::ZERO),
        nonce: Felt::ZERO,
        execute_after: 0,
        execute_before: 0,
        calls: vec![],
    })
}

fn craft_valid_outside_execution_calls(
    caller: ContractAddress,
    vrf_address: ContractAddress,
    calls_count: usize,
    first_call_selector: Option<Felt>,
    first_call_calldata: Option<Vec<Felt>>,
) -> Vec<Call> {
    let mut calls = vec![Call {
        to: vrf_address,
        selector: first_call_selector.unwrap_or(selector!("request_random")),
        calldata: first_call_calldata.unwrap_or(vec![caller.into(), Felt::ZERO, Felt::ONE]),
    }];
    for i in 0..calls_count {
        let hex_data = Felt::from_hex(&format!("0x{}", i)).unwrap();
        calls.push(Call {
            to: ContractAddress::from(hex_data),
            selector: get_selector_from_name(format!("do_something_{}", i).as_str()).unwrap(),
            calldata: vec![hex_data],
        });
    }
    calls
}

fn craft_valid_outside_execution_v2(
    caller: ContractAddress,
    vrf_address: ContractAddress,
    calls_count: usize,
    first_call_selector: Option<Felt>,
    first_call_calldata: Option<Vec<Felt>>,
) -> OutsideExecution {
    let calls = craft_valid_outside_execution_calls(
        caller,
        vrf_address,
        calls_count,
        first_call_selector,
        first_call_calldata,
    );

    OutsideExecution::V2(OutsideExecutionV2 {
        caller,
        nonce: Felt::TWO,
        execute_after: 10,
        execute_before: 20,
        calls,
    })
}

fn craft_valid_outside_execution_v3(
    caller: ContractAddress,
    vrf_address: ContractAddress,
    calls_count: usize,
    first_call_selector: Option<Felt>,
    first_call_calldata: Option<Vec<Felt>>,
) -> OutsideExecution {
    let calls = craft_valid_outside_execution_calls(
        caller,
        vrf_address,
        calls_count,
        first_call_selector,
        first_call_calldata,
    );

    OutsideExecution::V3(OutsideExecutionV3 {
        caller,
        nonce: NonceChannel::new(Felt::TWO, DEFAULT_NONCE_CHANNEL),
        execute_after: 10,
        execute_before: 20,
        calls,
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
    server: &mut mockito::Server,
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
async fn setup_cartridge_server() -> mockito::Server {
    mockito::Server::new_with_opts_async(Default::default()).await
}

// setup the mocks for the cartridge server
async fn setup_mocks(
    server: &mut mockito::Server,
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
) -> (TestNode, Paymaster<BlockifierFactory>, ContractAddress) {
    let config = test_config();
    let sequencer = TestNode::new_with_config(config.clone()).await;
    let block_producer = BlockProducer::instant(Arc::clone(&sequencer.backend()));
    let validator = block_producer.validator();
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
    let (pm_address, pm_account) = sequencer
        .backend()
        .chain_spec
        .genesis()
        .accounts()
        .nth(0)
        .expect("must have at least one account");

    let private_key = pm_account.private_key().expect("must exist");

    let vrf_ctx = VrfContext::new(VRF_PRIVATE_KEY_FOR_TESTS, *pm_address);
    let paymaster = Paymaster::new(
        starknet_api,
        client,
        TxPool::new(validator.clone(), FiFo::new()),
        ChainId::Id(Felt::ONE),
        paymaster_address.unwrap_or(*pm_address),
        paymaster_private_key.unwrap_or(SigningKey::from_secret_scalar(private_key)),
        vrf_ctx.clone(),
    );

    (sequencer, paymaster, vrf_ctx.address())
}

async fn deploy_vrf_provider(node: &TestNode, paymaster_address: ContractAddress) {
    let account = node.account();
    let provider = node.starknet_provider();

    let path = PathBuf::from(VRF_PROVIDER_CLASS_PATH);

    let (contract, compiled_class_hash) = prepare_contract_declaration_params(&path)
        .expect("failed to prepare VRF provider contract");

    let res = account
        .declare_v3(contract.into(), compiled_class_hash)
        .send()
        .await
        .expect("failed to send declare tx");

    katana_utils::TxWaiter::new(res.transaction_hash, &provider)
        .await
        .expect("failed to wait on tx");

    let tx = account
        .execute_v3(vec![StarknetCall {
            calldata: vec![
                CARTRIDGE_VRF_CLASS_HASH,
                CARTRIDGE_VRF_SALT,
                Felt::ZERO,
                Felt::THREE,
                paymaster_address.into(),
                VRF_PUBLIC_KEY_X,
                VRF_PUBLIC_KEY_Y,
            ],
            to: DEFAULT_UDC_ADDRESS.into(),
            selector: selector!("deployContract"),
        }])
        .send()
        .await
        .expect("failed to send execute tx");

    TxWaiter::new(tx.transaction_hash, &provider).await.expect("failed to wait on tx");
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

fn assert_outside_execution_v2(
    new_outside_execution: &OutsideExecution,
    fake_calls_count: usize,
    expected_nonce: Felt,
    vrf_address: ContractAddress,
    paymaster_address: ContractAddress,
) {
    assert_eq!(new_outside_execution.caller(), paymaster_address);

    if let OutsideExecution::V2(v2) = new_outside_execution {
        assert_eq!(v2.calls.len(), fake_calls_count + 3);

        assert_eq!(v2.nonce, expected_nonce);
        assert_eq!(v2.caller, paymaster_address);

        // first call should be submit random
        assert_eq!(v2.calls[0].to, vrf_address);
        assert_eq!(v2.calls[0].selector, selector!("submit_random"));
        assert_eq!(v2.calls[0].calldata.len(), 6);

        // second call should be the request_random from the initial outside execution
        assert_eq!(v2.calls[1].to, vrf_address);
        assert_eq!(v2.calls[1].selector, selector!("request_random"));
        assert_eq!(v2.calls[1].calldata.len(), 3);

        // then fake calls from the initial outside execution
        for i in 0..fake_calls_count {
            let call_index = i + 2;

            let hex_data = Felt::from_hex(&format!("0x{}", i)).unwrap();
            assert_eq!(v2.calls[call_index].to, ContractAddress::from(hex_data));
            assert_eq!(
                v2.calls[call_index].selector,
                get_selector_from_name(format!("do_something_{}", i).as_str()).unwrap()
            );
            assert_eq!(v2.calls[call_index].calldata, vec![hex_data]);
        }

        // finally, the last call should be assert_consumed
        let last_call_index = fake_calls_count + 2;
        assert_eq!(v2.calls[last_call_index].to, vrf_address);
        assert_eq!(v2.calls[last_call_index].selector, selector!("assert_consumed"));
        assert_eq!(v2.calls[last_call_index].calldata.len(), 1);
    } else {
        panic!("expected OutsideExecution::V2");
    }
}

fn assert_outside_execution_v3(
    new_outside_execution: &OutsideExecution,
    fake_calls_count: usize,
    expected_nonce: Felt,
    vrf_address: ContractAddress,
    paymaster_address: ContractAddress,
) {
    assert_eq!(new_outside_execution.caller(), paymaster_address);

    if let OutsideExecution::V3(v3) = new_outside_execution {
        assert_eq!(v3.calls.len(), fake_calls_count + 3);

        assert_eq!(v3.nonce, NonceChannel::new(expected_nonce, DEFAULT_NONCE_CHANNEL));
        assert_eq!(v3.caller, paymaster_address);

        // first call should be submit random
        assert_eq!(v3.calls[0].to, vrf_address);
        assert_eq!(v3.calls[0].selector, selector!("submit_random"));
        assert_eq!(v3.calls[0].calldata.len(), 6);

        // second call should be the request_random from the initial outside execution
        assert_eq!(v3.calls[1].to, vrf_address);
        assert_eq!(v3.calls[1].selector, selector!("request_random"));
        assert_eq!(v3.calls[1].calldata.len(), 3);

        // then fake calls from the initial outside execution
        for i in 0..fake_calls_count {
            let call_index = i + 2;

            let hex_data = Felt::from_hex(&format!("0x{}", i)).unwrap();
            assert_eq!(v3.calls[call_index].to, ContractAddress::from(hex_data));
            assert_eq!(
                v3.calls[call_index].selector,
                get_selector_from_name(format!("do_something_{}", i).as_str()).unwrap()
            );
            assert_eq!(v3.calls[call_index].calldata, vec![hex_data]);
        }

        // finally, the last call should be assert_consumed
        let last_call_index = fake_calls_count + 2;
        assert_eq!(v3.calls[last_call_index].to, vrf_address);
        assert_eq!(v3.calls[last_call_index].selector, selector!("assert_consumed"));
        assert_eq!(v3.calls[last_call_index].calldata.len(), 1);
    } else {
        panic!("expected OutsideExecution::V3");
    }
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

fn assert_controller_deploy_tx(
    tx: &ExecutableTx,
    paymaster_address: ContractAddress,
    address: &str,
) {
    if let ExecutableTx::Invoke(InvokeTx::V3(tx)) = tx {
        assert_eq!(tx.sender_address, paymaster_address, "bad sender address");
        assert_eq!(tx.nonce, Felt::ZERO, "bad nonce");
        assert_eq!(tx.calldata[0], Felt::ONE, "bad number of calls");
        assert_eq!(tx.calldata[1], DEFAULT_UDC_ADDRESS.into(), "bad UDC address");
        assert_eq!(tx.calldata[2], selector!("deployContract"), "bad selector");

        let ctor_data = build_calldata(address)
            .iter()
            .map(|s| Felt::from_hex(s).unwrap())
            .collect::<Vec<Felt>>();

        assert_eq!(tx.calldata[3], ctor_data.len().into(), "bad calldata length");
        assert_eq!(tx.calldata[4..], ctor_data, "bad calldata");
    } else {
        panic!("expected invoke tx v3");
    }
}

fn assert_vrf_provider_deploy_tx(
    tx: &ExecutableTx,
    paymaster_address: ContractAddress,
    nonce: Felt,
) {
    if let ExecutableTx::Invoke(InvokeTx::V3(tx)) = tx {
        assert_eq!(tx.sender_address, paymaster_address, "bad sender address");
        assert_eq!(tx.nonce, nonce, "bad nonce");
        assert_eq!(tx.calldata[0], Felt::ONE, "bad number of calls");
        assert_eq!(tx.calldata[1], DEFAULT_UDC_ADDRESS.into(), "bad UDC address");
        assert_eq!(tx.calldata[2], selector!("deployContract"), "bad selector");

        assert_eq!(tx.calldata[3], Felt::from_hex("0x07").unwrap(), "bad calldata length");
        assert_eq!(tx.calldata[4], CARTRIDGE_VRF_CLASS_HASH, "bad VRF class hash");
        assert_eq!(tx.calldata[5], CARTRIDGE_VRF_SALT, "bad VRF salt");
        assert_eq!(tx.calldata[6], Felt::ZERO, "bad from zero");
        assert_eq!(tx.calldata[7], Felt::THREE, "bad calldata length");
        assert_eq!(tx.calldata[8], paymaster_address.into(), "bad paymaster address");
        assert_eq!(tx.calldata[9], VRF_PUBLIC_KEY_X, "bad public key x");
        assert_eq!(tx.calldata[10], VRF_PUBLIC_KEY_Y, "bad public key y");
    } else {
        panic!("expected invoke tx v3");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_starknet_estimate_fee_without_txs() {
    let server = setup_cartridge_server().await;
    let (_, paymaster, _) = setup(&server.url(), None, None).await;

    let res = paymaster.handle_estimate_fees(BlockIdOrTag::Tag(BlockTag::Pending), vec![]).await;

    assert!(res.is_ok());
    assert_eq!(res.unwrap().len(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_starknet_estimate_fee_when_sender_is_not_a_controller() {
    let mut server = setup_cartridge_server().await;
    let (_, paymaster, _) = setup(&server.url(), None, None).await;

    let mocks =
        setup_mocks(&mut server, &[(&CONTROLLER_ADDRESS_1.to_hex_string(), false, true)]).await;

    let sender_address = CONTROLLER_ADDRESS_1.into();
    let tx = invoke_tx(sender_address);

    let res = paymaster.handle_estimate_fees(BlockIdOrTag::Tag(BlockTag::Pending), vec![tx]).await;

    assert!(res.is_ok());
    assert_eq!(res.unwrap().len(), 0);
    assert_mocks(&mocks);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_starknet_estimate_fee_when_sender_is_an_already_deployed_controller() {
    let mut server = setup_cartridge_server().await;
    let (_, paymaster, _) = setup(&server.url(), None, None).await;

    let mocks = setup_mocks(
        &mut server,
        &[(&ALREADY_DEPLOYED_CONTROLLER_ADDRESS.to_hex_string(), true, false)],
    )
    .await;

    let tx = invoke_tx(ALREADY_DEPLOYED_CONTROLLER_ADDRESS.into());
    let res = paymaster.handle_estimate_fees(BlockIdOrTag::Tag(BlockTag::Pending), vec![tx]).await;

    assert!(res.is_ok());
    assert_eq!(res.unwrap().len(), 0);

    assert_mocks(&mocks);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_starknet_estimate_fee_when_sender_is_a_not_yet_deployed_controller() {
    let mut server = setup_cartridge_server().await;
    let (_, paymaster, _) = setup(&server.url(), None, None).await;

    let mocks =
        setup_mocks(&mut server, &[(&CONTROLLER_ADDRESS_1.to_hex_string(), true, true)]).await;

    let sender_address = ContractAddress::from(CONTROLLER_ADDRESS_1);
    let tx = invoke_tx(sender_address);

    let res = paymaster.handle_estimate_fees(BlockIdOrTag::Tag(BlockTag::Pending), vec![tx]).await;

    assert!(res.is_ok());
    let res = res.unwrap();
    assert_eq!(res.len(), 1);
    assert_tx(&res[0], &CONTROLLER_ADDRESS_1.to_hex_string());

    assert_mocks(&mocks);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_starknet_estimate_fee_when_several_txs_with_the_same_not_yet_deployed_controller() {
    let mut server = setup_cartridge_server().await;
    let (_, paymaster, _) = setup(&server.url(), None, None).await;

    let mocks =
        setup_mocks(&mut server, &[(&CONTROLLER_ADDRESS_1.to_hex_string(), true, true)]).await;

    let sender_address = ContractAddress::from(CONTROLLER_ADDRESS_1);
    let txs = vec![invoke_tx(sender_address), invoke_tx(sender_address)];

    let res = paymaster.handle_estimate_fees(BlockIdOrTag::Tag(BlockTag::Pending), txs).await;

    assert!(res.is_ok());

    let res = res.unwrap();
    assert_eq!(res.len(), 1);
    assert_tx(&res[0], &CONTROLLER_ADDRESS_1.to_hex_string());
    assert_mocks(&mocks);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_starknet_estimate_fee_when_several_txs_with_several_controllers() {
    let mut server = setup_cartridge_server().await;
    let (_, paymaster, _) = setup(&server.url(), None, None).await;

    let mocks = setup_mocks(
        &mut server,
        &[
            (&CONTROLLER_ADDRESS_1.to_hex_string(), true, true),
            (&CONTROLLER_ADDRESS_2.to_hex_string(), true, true),
        ],
    )
    .await;

    let txs = vec![
        invoke_tx(ContractAddress::from(CONTROLLER_ADDRESS_1)),
        invoke_tx(ContractAddress::from(CONTROLLER_ADDRESS_2)),
    ];

    let res = paymaster.handle_estimate_fees(BlockIdOrTag::Tag(BlockTag::Pending), txs).await;

    assert!(res.is_ok());

    let res = res.unwrap();
    assert_eq!(res.len(), 2);
    assert_tx(&res[0], &CONTROLLER_ADDRESS_1.to_hex_string());
    assert_tx(&res[1], &CONTROLLER_ADDRESS_2.to_hex_string());

    assert_mocks(&mocks);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_starknet_estimate_fee_sender_with_invalid_paymaster_config() {
    let mut server = setup_cartridge_server().await;

    // configure a wrong paymaster address
    let wrong_paymaster_address = ContractAddress::from(Felt::THREE);
    let (_, paymaster, _) = setup(&server.url(), Some(wrong_paymaster_address), None).await;

    let mocks =
        setup_mocks(&mut server, &[(&CONTROLLER_ADDRESS_1.to_hex_string(), true, false)]).await;

    let sender_address = ContractAddress::from(CONTROLLER_ADDRESS_1);
    let tx = invoke_tx(sender_address);

    let res = paymaster.handle_estimate_fees(BlockIdOrTag::Tag(BlockTag::Pending), vec![tx]).await;

    assert!(res.is_err());
    assert_matches!(res.unwrap_err(), Error::PaymasterNotFound(_));
    assert_mocks(&mocks);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_starknet_estimate_fee_with_a_mix_of_txs() {
    let mut server = setup_cartridge_server().await;
    let (_, paymaster, _) = setup(&server.url(), None, None).await;

    let mocks = setup_mocks(
        &mut server,
        &[
            ("0x12345", false, false),
            ("0x67890", false, false),
            (&CONTROLLER_ADDRESS_1.to_hex_string(), true, true),
            (&CONTROLLER_ADDRESS_2.to_hex_string(), true, true),
        ],
    )
    .await;

    let txs = vec![
        invoke_tx(address!("0x67890")),
        invoke_tx(ContractAddress::from(CONTROLLER_ADDRESS_2)),
        invoke_tx(address!("0x12345")),
        invoke_tx(address!("0x67890")),
        invoke_tx(ContractAddress::from(CONTROLLER_ADDRESS_1)),
        invoke_tx(ContractAddress::from(CONTROLLER_ADDRESS_2)),
    ];

    let res = paymaster.handle_estimate_fees(BlockIdOrTag::Tag(BlockTag::Pending), txs).await;

    assert!(res.is_ok());

    let res = res.unwrap();
    assert_eq!(res.len(), 2);
    assert_tx(&res[0], &CONTROLLER_ADDRESS_2.to_hex_string());
    assert_tx(&res[1], &CONTROLLER_ADDRESS_1.to_hex_string());

    assert_mocks(&mocks);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_cartridge_outside_execution_when_caller_is_not_a_controller() {
    let mut server = setup_cartridge_server().await;
    let (_, paymaster, _) = setup(&server.url(), None, None).await;

    let mocks =
        setup_mocks(&mut server, &[(&CONTROLLER_ADDRESS_1.to_hex_string(), false, true)]).await;

    let res = paymaster
        .handle_add_outside_execution(
            ContractAddress::from(CONTROLLER_ADDRESS_1),
            default_outside_execution(),
            vec![],
        )
        .await;

    assert!(res.is_ok());
    assert!(res.unwrap().is_none());

    assert_eq!(paymaster.pool.size(), 0);

    assert_mocks(&mocks);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_cartridge_outside_execution_when_caller_is_an_already_deployed_controller() {
    let mut server = setup_cartridge_server().await;
    let (node, paymaster, _) = setup(&server.url(), None, None).await;

    let (caller_address, _) = node
        .backend()
        .chain_spec
        .genesis()
        .accounts()
        .nth(0)
        .expect("must have at least one account");

    let mocks = setup_mocks(&mut server, &[(&caller_address.to_hex_string(), true, false)]).await;

    let res = paymaster
        .handle_add_outside_execution(*caller_address, default_outside_execution(), vec![])
        .await;

    assert!(res.is_ok());
    assert!(res.unwrap().is_none());

    assert_eq!(paymaster.pool.size(), 0);

    assert_mocks(&mocks);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_cartridge_outside_execution_when_caller_is_a_not_yet_deployed_controller() {
    let mut server = setup_cartridge_server().await;
    let (_, paymaster, _) = setup(&server.url(), None, None).await;

    let mocks =
        setup_mocks(&mut server, &[(&CONTROLLER_ADDRESS_2.to_hex_string(), true, true)]).await;

    let res = paymaster
        .handle_add_outside_execution(
            ContractAddress::from(CONTROLLER_ADDRESS_2),
            default_outside_execution(),
            vec![],
        )
        .await;

    assert!(res.is_ok());
    assert!(res.unwrap().is_none());

    assert_eq!(paymaster.pool.size(), 1);

    let tx = &paymaster.pool.pending_transactions().next().await.unwrap().tx.transaction;
    assert_controller_deploy_tx(
        tx,
        paymaster.paymaster_address,
        &CONTROLLER_ADDRESS_2.to_hex_string(),
    );

    assert_mocks(&mocks);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_cartridge_outside_execution_when_vrf_is_not_deployed() {
    let mut server = setup_cartridge_server().await;
    let (_, paymaster, vrf_address) = setup(&server.url(), None, None).await;

    let mocks =
        setup_mocks(&mut server, &[(&CONTROLLER_ADDRESS_2.to_hex_string(), true, true)]).await;

    let fake_calls_count = 3;
    let outside_execution = craft_valid_outside_execution_v2(
        ContractAddress::from(CONTROLLER_ADDRESS_2),
        vrf_address,
        fake_calls_count,
        None,
        None,
    );

    let res = paymaster
        .handle_add_outside_execution(
            ContractAddress::from(CONTROLLER_ADDRESS_2),
            outside_execution,
            vec![],
        )
        .await;

    assert!(res.is_ok());
    assert_outside_execution_v2(
        &res.unwrap().unwrap().0,
        fake_calls_count,
        Felt::TWO,
        vrf_address,
        paymaster.paymaster_address,
    );

    assert_eq!(paymaster.pool.size(), 2);

    let mut pending_transactions = paymaster.pool.pending_transactions();

    let tx = pending_transactions.next().await.unwrap().tx.transaction.clone();
    assert_controller_deploy_tx(
        &tx,
        paymaster.paymaster_address,
        &CONTROLLER_ADDRESS_2.to_hex_string(),
    );

    let tx = pending_transactions.next().await.unwrap().tx.transaction.clone();
    assert_vrf_provider_deploy_tx(&tx, paymaster.paymaster_address, Felt::ONE);

    assert_mocks(&mocks);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_cartridge_outside_execution_when_vrf_is_already_deployed() {
    let mut server = setup_cartridge_server().await;
    let (node, paymaster, vrf_address) = setup(&server.url(), None, None).await;

    // use an already deployed account for the controller
    let (controller_address, _) = node
        .backend()
        .chain_spec
        .genesis()
        .accounts()
        .nth(0)
        .expect("must have at least one account");

    // deploy the VRF provider
    deploy_vrf_provider(&node, paymaster.paymaster_address).await;

    let mocks =
        setup_mocks(&mut server, &[(&controller_address.to_hex_string(), true, true)]).await;

    let fake_calls_count = 3;
    let outside_execution = craft_valid_outside_execution_v3(
        *controller_address,
        vrf_address,
        fake_calls_count,
        None,
        None,
    );

    let res = paymaster
        .handle_add_outside_execution(*controller_address, outside_execution, vec![])
        .await;

    assert!(res.is_ok());
    assert_outside_execution_v3(
        &res.unwrap().unwrap().0,
        fake_calls_count,
        Felt::ZERO,
        vrf_address,
        paymaster.paymaster_address,
    );

    assert_eq!(paymaster.pool.size(), 0);
    assert_mocks(&mocks);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_cartridge_outside_execution_when_first_call_is_not_request_random() {
    let mut server = setup_cartridge_server().await;
    let (_, paymaster, vrf_address) = setup(&server.url(), None, None).await;

    let mocks =
        setup_mocks(&mut server, &[(&CONTROLLER_ADDRESS_2.to_hex_string(), true, true)]).await;

    let fake_calls_count = 3;
    let outside_execution = craft_valid_outside_execution_v3(
        CONTROLLER_ADDRESS_2.into(),
        vrf_address,
        fake_calls_count,
        Some(selector!("not_request_random")),
        None,
    );

    let res = paymaster
        .handle_add_outside_execution(CONTROLLER_ADDRESS_2.into(), outside_execution, vec![])
        .await;

    assert!(res.is_ok());
    assert!(res.unwrap().is_none());

    assert_mocks(&mocks);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_cartridge_outside_execution_when_vrf_not_targeting_vrf_provider() {
    let mut server = setup_cartridge_server().await;
    let (_, paymaster, _) = setup(&server.url(), None, None).await;

    let mocks =
        setup_mocks(&mut server, &[(&CONTROLLER_ADDRESS_2.to_hex_string(), true, true)]).await;

    let fake_calls_count = 3;
    let outside_execution = craft_valid_outside_execution_v3(
        CONTROLLER_ADDRESS_2.into(),
        CONTROLLER_ADDRESS_1.into(), // not targeting VRF provider
        fake_calls_count,
        None,
        None,
    );

    let res = paymaster
        .handle_add_outside_execution(CONTROLLER_ADDRESS_2.into(), outside_execution, vec![])
        .await;

    // when 'request_random' is not targeting the VRF provider, the request is simply ignored,
    // as it might be a valid call to another contract.
    assert!(res.is_ok());
    assert!(res.unwrap().is_none());

    assert_mocks(&mocks);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_cartridge_outside_execution_when_vrf_bad_calldata() {
    let mut server = setup_cartridge_server().await;
    let (_, paymaster, vrf_address) = setup(&server.url(), None, None).await;

    let mocks =
        setup_mocks(&mut server, &[(&CONTROLLER_ADDRESS_2.to_hex_string(), true, true)]).await;

    let fake_calls_count = 3;
    let outside_execution = craft_valid_outside_execution_v3(
        CONTROLLER_ADDRESS_2.into(),
        vrf_address,
        fake_calls_count,
        None,
        Some(vec![Felt::ONE]),
    );

    let res = paymaster
        .handle_add_outside_execution(CONTROLLER_ADDRESS_2.into(), outside_execution, vec![])
        .await;

    assert!(res.is_err());
    assert_matches!(res.unwrap_err(), Error::Vrf(e) if e.contains("Invalid calldata for request_random"));

    assert_mocks(&mocks);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_cartridge_outside_execution_when_vrf_bad_source() {
    let mut server = setup_cartridge_server().await;
    let (_, paymaster, vrf_address) = setup(&server.url(), None, None).await;

    let mocks =
        setup_mocks(&mut server, &[(&CONTROLLER_ADDRESS_2.to_hex_string(), true, true)]).await;

    let fake_calls_count = 3;
    let outside_execution = craft_valid_outside_execution_v3(
        CONTROLLER_ADDRESS_2.into(),
        vrf_address,
        fake_calls_count,
        None,
        Some(vec![CONTROLLER_ADDRESS_2, Felt::TWO, Felt::ONE]),
    );

    let res = paymaster
        .handle_add_outside_execution(CONTROLLER_ADDRESS_2.into(), outside_execution, vec![])
        .await;

    assert!(res.is_err());
    assert_matches!(res.unwrap_err(), Error::Vrf(e) if e.contains("Invalid salt or nonce for VRF request"));

    assert_mocks(&mocks);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_cartridge_outside_execution_when_vrf_request_random_call_only() {
    let mut server = setup_cartridge_server().await;
    let (_, paymaster, vrf_address) = setup(&server.url(), None, None).await;

    let mocks =
        setup_mocks(&mut server, &[(&CONTROLLER_ADDRESS_2.to_hex_string(), true, true)]).await;

    let outside_execution =
        craft_valid_outside_execution_v3(CONTROLLER_ADDRESS_2.into(), vrf_address, 0, None, None);

    let res = paymaster
        .handle_add_outside_execution(CONTROLLER_ADDRESS_2.into(), outside_execution, vec![])
        .await;

    assert!(res.is_ok());
    assert!(res.unwrap().is_none());

    assert_mocks(&mocks);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_cartridge_outside_execution_when_caller_is_a_not_yet_deployed_but_wrong_paymaster_pkey(
) {
    let mut server = setup_cartridge_server().await;
    let (_, paymaster, _) =
        setup(&server.url(), None, Some(SigningKey::from_secret_scalar(Felt::THREE))).await;

    let mocks =
        setup_mocks(&mut server, &[(&CONTROLLER_ADDRESS_2.to_hex_string(), true, true)]).await;

    let res = paymaster
        .handle_add_outside_execution(
            ContractAddress::from(CONTROLLER_ADDRESS_2),
            default_outside_execution(),
            vec![],
        )
        .await;

    assert!(res.is_err());
    assert_matches!(res.unwrap_err(), Error::FailedToAddTransaction(_));
    assert_mocks(&mocks);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_cartridge_outside_execution_when_paymaster_is_not_properly_configured() {
    let mut server = setup_cartridge_server().await;
    let (_, paymaster, _) =
        setup(&server.url(), Some(ContractAddress::from(Felt::THREE)), None).await;

    let mocks =
        setup_mocks(&mut server, &[(&CONTROLLER_ADDRESS_1.to_hex_string(), true, false)]).await;

    let res = paymaster
        .handle_add_outside_execution(
            ContractAddress::from(CONTROLLER_ADDRESS_1),
            default_outside_execution(),
            vec![],
        )
        .await;

    assert!(res.is_err());
    assert_matches!(res.unwrap_err(), Error::PaymasterNotFound(_));
    assert_mocks(&mocks);
}

// TODO:
// - check VRF mechanism with nonce storage
