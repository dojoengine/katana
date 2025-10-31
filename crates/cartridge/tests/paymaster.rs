// Tests for the paymaster:
// - when starknet_estimateFee is called, for each tx of the batch the paymaster should:
//    + check if the sender_address is a controller acccount
//    + deploy the controller account if not already deployed.
// - when cartridge_addExecuteOutsideTransaction is called, the paymaster should deploy
//   the controller account if not already deployed.

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

fn build_response(address: &str, is_controller: bool) -> String {
    if !is_controller {
        return "Address not found".to_string();
    }

    json!({
        "address": address,
        "username": "user",
        "calldata": ["0x01", "0x02", "0x03"]
    })
    .to_string()
}

async fn setup_cartridge(addresses: &[(&str, bool)]) -> (mockito::ServerGuard, Vec<mockito::Mock>) {
    let mut server = mockito::Server::new_async().await;
    let mut mocks: Vec<mockito::Mock> = Vec::new();

    for (address, is_controller) in addresses {
        let expected_call_count = if *is_controller { 1 } else { 0 };
        mocks.push(
            server
                .mock("POST", "/accounts/calldata")
                .match_body(mockito::Matcher::Regex(address.to_string()))
                .with_body(build_response(address, *is_controller))
                .expect_at_least(expected_call_count)
                .create_async()
                .await,
        );
    }

    (server, mocks)
}

/// Setup the test node and the paymaster.
async fn setup(
    cartridge_url: &str,
    paymaster_address: Option<ContractAddress>,
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
    let (account_address, _) = sequencer
        .backend()
        .chain_spec
        .genesis()
        .accounts()
        .nth(0)
        .expect("must have at least one account");

    let paymaster = Paymaster::new(
        starknet_api,
        client,
        TxPool::new(validator.clone(), FiFo::new()),
        ChainId::Id(Felt::ONE),
        paymaster_address.unwrap_or(*account_address),
        SigningKey::from_secret_scalar(Felt::ZERO),
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

#[tokio::test(flavor = "multi_thread")]
async fn test_starknet_estimate_fee_without_txs() {
    let (server, _) = setup_cartridge(&[]).await;
    let (_, paymaster) = setup(&server.url(), None).await;

    let res = paymaster.handle_estimate_fees(BlockIdOrTag::Tag(BlockTag::Pending), vec![]);

    assert!(res.is_ok());
    assert_eq!(res.unwrap().len(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_starknet_estimate_fee_when_sender_is_not_a_controller() {
    let (server, mocks) = setup_cartridge(&[(CONTROLLER_ADDRESS_1, false)]).await;
    let (_, paymaster) = setup(&server.url(), None).await;

    let sender_address = ContractAddress::from(Felt::from_hex(CONTROLLER_ADDRESS_1).unwrap());
    let tx = invoke_tx(sender_address);

    let res = paymaster.handle_estimate_fees(BlockIdOrTag::Tag(BlockTag::Pending), vec![tx]);

    assert!(res.is_ok());
    assert_eq!(res.unwrap().len(), 0);
    mocks.iter().for_each(|mock| mock.assert());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_starknet_estimate_fee_when_sender_is_an_already_deployed_controller() {
    let (server, _) = setup_cartridge(&[(CONTROLLER_ADDRESS_1, true)]).await;
    let (node, paymaster) = setup(&server.url(), None).await;

    let (sender_address, _) = node
        .backend()
        .chain_spec
        .genesis()
        .accounts()
        .nth(0)
        .expect("must have at least one account");

    let tx = invoke_tx(*sender_address);
    let res = paymaster.handle_estimate_fees(BlockIdOrTag::Tag(BlockTag::Pending), vec![tx]);

    assert!(res.is_ok());
    assert_eq!(res.unwrap().len(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_starknet_estimate_fee_when_sender_is_a_not_yet_deployed_controller() {
    let (server, mocks) = setup_cartridge(&[(CONTROLLER_ADDRESS_1, true)]).await;
    let (_, paymaster) = setup(&server.url(), None).await;

    let sender_address = ContractAddress::from(Felt::from_hex(CONTROLLER_ADDRESS_1).unwrap());
    let tx = invoke_tx(sender_address);

    let res = paymaster.handle_estimate_fees(BlockIdOrTag::Tag(BlockTag::Pending), vec![tx]);

    assert!(res.is_ok());
    assert_eq!(res.unwrap().len(), 1);
    mocks.iter().for_each(|mock| mock.assert());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_starknet_estimate_fee_when_several_txs_with_the_same_not_yet_deployed_controller() {
    let (server, mocks) = setup_cartridge(&[(CONTROLLER_ADDRESS_1, true)]).await;
    let (_, paymaster) = setup(&server.url(), None).await;

    let sender_address = ContractAddress::from(Felt::from_hex(CONTROLLER_ADDRESS_1).unwrap());
    let txs = vec![invoke_tx(sender_address), invoke_tx(sender_address)];

    let res = paymaster.handle_estimate_fees(BlockIdOrTag::Tag(BlockTag::Pending), txs);

    assert!(res.is_ok());
    assert_eq!(res.unwrap().len(), 1);
    mocks.iter().for_each(|mock| mock.assert());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_starknet_estimate_fee_when_several_txs_with_several_controllers() {
    let (server, mocks) =
        setup_cartridge(&[(CONTROLLER_ADDRESS_1, true), (CONTROLLER_ADDRESS_2, true)]).await;
    let (_, paymaster) = setup(&server.url(), None).await;

    let txs = vec![
        invoke_tx(ContractAddress::from(Felt::from_hex(CONTROLLER_ADDRESS_1).unwrap())),
        invoke_tx(ContractAddress::from(Felt::from_hex(CONTROLLER_ADDRESS_2).unwrap())),
    ];

    let res = paymaster.handle_estimate_fees(BlockIdOrTag::Tag(BlockTag::Pending), txs);

    assert!(res.is_ok());
    assert_eq!(res.unwrap().len(), 2);
    mocks.iter().for_each(|mock| mock.assert());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_starknet_estimate_fee_sender_with_invalid_paymaster_config() {
    let (server, _) = setup_cartridge(&[(CONTROLLER_ADDRESS_1, true)]).await;

    // configure a wrong paymaster address
    let wrong_paymaster_address = ContractAddress::from(Felt::THREE);
    let (_, paymaster) = setup(&server.url(), Some(wrong_paymaster_address)).await;

    let sender_address = ContractAddress::from(Felt::from_hex(CONTROLLER_ADDRESS_1).unwrap());
    let tx = invoke_tx(sender_address);

    let res = paymaster.handle_estimate_fees(BlockIdOrTag::Tag(BlockTag::Pending), vec![tx]);

    assert!(res.is_err());
    assert_matches!(res.unwrap_err(), Error::PaymasterNotFound(_));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_starknet_estimate_fee_with_a_mix_of_txs() {
    let (server, mocks) = setup_cartridge(&[
        ("0x12345", false),
        ("0x67890", false),
        (CONTROLLER_ADDRESS_1, true),
        (CONTROLLER_ADDRESS_2, true),
    ])
    .await;
    let (_, paymaster) = setup(&server.url(), None).await;

    let txs = vec![
        invoke_tx(ContractAddress::from(Felt::from_hex("0x67890").unwrap())),
        invoke_tx(ContractAddress::from(Felt::from_hex(CONTROLLER_ADDRESS_2).unwrap())),
        invoke_tx(ContractAddress::from(Felt::from_hex("0x12345").unwrap())),
        invoke_tx(ContractAddress::from(Felt::from_hex("0x67890").unwrap())),
        invoke_tx(ContractAddress::from(Felt::from_hex(CONTROLLER_ADDRESS_1).unwrap())),
        invoke_tx(ContractAddress::from(Felt::from_hex(CONTROLLER_ADDRESS_2).unwrap())),
    ];

    let res = paymaster.handle_estimate_fees(BlockIdOrTag::Tag(BlockTag::Pending), txs);

    println!("res: {:?}", res);
    assert!(res.is_ok());
    assert_eq!(res.unwrap().len(), 2);
    mocks.iter().for_each(|mock| mock.assert());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_cartridge_add_execute_outside_transaction_handling() {

    // 1. the caller is not a controller account => nothing special
    // 2. the caller is a controller account already deployed => nothing special
    // 3. the caller is a controller account not already deployed => deploy it
    // 4. paymaster not properly configured => error

    //    paymaster.handle_add_outside_execution(ContractAddress::new(0));
}
