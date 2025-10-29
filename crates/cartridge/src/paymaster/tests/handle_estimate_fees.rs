use assert_matches::assert_matches;
use katana_primitives::block::BlockIdOrTag;
use katana_primitives::contract::ContractAddress;
use katana_primitives::transaction::ExecutableTx;
use katana_primitives::{address, Felt};
use katana_rpc_types::BroadcastedTxWithChainId;

use super::utils::{
    assert_mocks, assert_tx, assert_vrf_provider_deploy_tx, invoke_tx, setup_cartridge_server,
    setup_mocks, ALREADY_DEPLOYED_CONTROLLER_ADDRESS, CONTROLLER_ADDRESS_1, CONTROLLER_ADDRESS_2,
};
use crate::paymaster::tests::utils::{deploy_vrf_provider, setup};
use crate::paymaster::Error;

#[tokio::test(flavor = "multi_thread")]
async fn test_starknet_estimate_fee_without_txs_should_deploy_vrf_provider() {
    let server = setup_cartridge_server().await;
    let (_, paymaster, _) = setup(&server.url(), None, None).await;

    let txs = vec![];
    let res = paymaster.handle_estimate_fees(BlockIdOrTag::PreConfirmed, &txs).await;

    assert!(res.is_ok());
    let res = res.unwrap();

    assert!(res.is_some());
    let txs = res.unwrap();
    assert_eq!(txs.len(), 1);

    let tx: ExecutableTx =
        BroadcastedTxWithChainId { tx: txs[0].clone(), chain: paymaster.chain_id }.into();
    assert_vrf_provider_deploy_tx(&tx, paymaster.paymaster_address, Felt::ZERO);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_starknet_estimate_fee_without_txs_vrf_already_deployed() {
    let server = setup_cartridge_server().await;
    let (node, paymaster, _) = setup(&server.url(), None, None).await;

    deploy_vrf_provider(&node, paymaster.paymaster_address).await;

    let txs = vec![];
    let res = paymaster.handle_estimate_fees(BlockIdOrTag::PreConfirmed, &txs).await;

    assert!(res.is_ok());
    assert!(res.unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_starknet_estimate_fee_when_sender_is_not_a_controller() {
    let mut server = setup_cartridge_server().await;
    let (node, paymaster, _) = setup(&server.url(), None, None).await;

    deploy_vrf_provider(&node, paymaster.paymaster_address).await;

    let mocks =
        setup_mocks(&mut server, &[(&CONTROLLER_ADDRESS_1.to_hex_string(), false, true)]).await;

    let sender_address = CONTROLLER_ADDRESS_1.into();
    let tx = invoke_tx(sender_address);

    let txs = vec![tx];
    let res = paymaster.handle_estimate_fees(BlockIdOrTag::PreConfirmed, &txs).await;

    assert!(res.is_ok());
    assert!(res.unwrap().is_none());
    assert_mocks(&mocks);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_starknet_estimate_fee_when_sender_is_an_already_deployed_controller() {
    let mut server = setup_cartridge_server().await;
    let (node, paymaster, _) = setup(&server.url(), None, None).await;

    deploy_vrf_provider(&node, paymaster.paymaster_address).await;

    let mocks = setup_mocks(
        &mut server,
        &[(&ALREADY_DEPLOYED_CONTROLLER_ADDRESS.to_hex_string(), true, false)],
    )
    .await;

    let tx = invoke_tx(ALREADY_DEPLOYED_CONTROLLER_ADDRESS.into());
    let txs = vec![tx];
    let res = paymaster.handle_estimate_fees(BlockIdOrTag::PreConfirmed, &txs).await;

    assert!(res.is_ok());
    assert!(res.unwrap().is_none());

    assert_mocks(&mocks);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_starknet_estimate_fee_when_sender_is_a_not_yet_deployed_controller() {
    let mut server = setup_cartridge_server().await;
    let (node, paymaster, _) = setup(&server.url(), None, None).await;

    deploy_vrf_provider(&node, paymaster.paymaster_address).await;

    let mocks =
        setup_mocks(&mut server, &[(&CONTROLLER_ADDRESS_1.to_hex_string(), true, true)]).await;

    let sender_address = ContractAddress::from(CONTROLLER_ADDRESS_1);
    let tx = invoke_tx(sender_address);

    let txs = vec![tx];
    let res = paymaster.handle_estimate_fees(BlockIdOrTag::PreConfirmed, &txs).await;

    assert!(res.is_ok());
    let res = res.unwrap().unwrap();
    assert_eq!(res.len(), 2);
    assert_tx(&res[0], &CONTROLLER_ADDRESS_1.to_hex_string());

    assert_mocks(&mocks);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_starknet_estimate_fee_when_several_txs_with_the_same_not_yet_deployed_controller() {
    let mut server = setup_cartridge_server().await;
    let (node, paymaster, _) = setup(&server.url(), None, None).await;

    deploy_vrf_provider(&node, paymaster.paymaster_address).await;

    let mocks =
        setup_mocks(&mut server, &[(&CONTROLLER_ADDRESS_1.to_hex_string(), true, true)]).await;

    let sender_address = ContractAddress::from(CONTROLLER_ADDRESS_1);
    let txs = vec![invoke_tx(sender_address), invoke_tx(sender_address)];

    let res = paymaster.handle_estimate_fees(BlockIdOrTag::PreConfirmed, &txs).await;

    assert!(res.is_ok());

    let res = res.unwrap().unwrap();
    assert_eq!(res.len(), 3);
    assert_tx(&res[0], &CONTROLLER_ADDRESS_1.to_hex_string());
    assert_mocks(&mocks);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_starknet_estimate_fee_when_several_txs_with_several_controllers() {
    let mut server = setup_cartridge_server().await;
    let (node, paymaster, _) = setup(&server.url(), None, None).await;

    deploy_vrf_provider(&node, paymaster.paymaster_address).await;

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

    let res = paymaster.handle_estimate_fees(BlockIdOrTag::PreConfirmed, &txs).await;

    assert!(res.is_ok());

    let res = res.unwrap().unwrap();
    assert_eq!(res.len(), 4);
    assert_tx(&res[0], &CONTROLLER_ADDRESS_1.to_hex_string());
    assert_tx(&res[1], &CONTROLLER_ADDRESS_2.to_hex_string());

    assert_mocks(&mocks);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_starknet_estimate_fee_sender_with_invalid_paymaster_config() {
    let mut server = setup_cartridge_server().await;

    // configure a wrong paymaster address
    let wrong_paymaster_address = ContractAddress::from(Felt::THREE);
    let (node, paymaster, _) = setup(&server.url(), Some(wrong_paymaster_address), None).await;

    deploy_vrf_provider(&node, paymaster.paymaster_address).await;

    let mocks =
        setup_mocks(&mut server, &[(&CONTROLLER_ADDRESS_1.to_hex_string(), true, false)]).await;

    let sender_address = ContractAddress::from(CONTROLLER_ADDRESS_1);
    let tx = invoke_tx(sender_address);

    let txs = vec![tx];
    let res = paymaster.handle_estimate_fees(BlockIdOrTag::PreConfirmed, &txs).await;

    assert!(res.is_err());
    assert_matches!(res.unwrap_err(), Error::PaymasterNotFound(_));
    assert_mocks(&mocks);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_starknet_estimate_fee_with_a_mix_of_txs() {
    let mut server = setup_cartridge_server().await;
    let (node, paymaster, _) = setup(&server.url(), None, None).await;

    deploy_vrf_provider(&node, paymaster.paymaster_address).await;

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

    let res = paymaster.handle_estimate_fees(BlockIdOrTag::PreConfirmed, &txs).await;

    assert!(res.is_ok());

    let res = res.unwrap().unwrap();
    assert_eq!(res.len(), 8);
    assert_tx(&res[0], &CONTROLLER_ADDRESS_2.to_hex_string());
    assert_tx(&res[1], &CONTROLLER_ADDRESS_1.to_hex_string());

    assert_mocks(&mocks);
}
