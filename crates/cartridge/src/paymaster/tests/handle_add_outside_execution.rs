use assert_matches::assert_matches;
use futures::StreamExt;
use katana_pool::TransactionPool;
use katana_primitives::contract::ContractAddress;
use katana_primitives::hash::{Poseidon, StarkHash};
use katana_primitives::Felt;
use katana_provider::api::state::StateWriter;
use katana_provider::{MutableProvider, ProviderFactory};
use starknet::macros::{felt, selector};
use starknet::signers::SigningKey;
use starknet_types_core::hash::Pedersen;

use super::utils::{
    assert_controller_deploy_tx, assert_crafted_outside_execution, assert_mocks,
    assert_vrf_provider_deploy_tx, craft_valid_outside_execution_v2,
    craft_valid_outside_execution_v3, default_outside_execution, deploy_vrf_provider, setup,
    setup_cartridge_server, setup_mocks, CONTROLLER_ADDRESS_1, CONTROLLER_ADDRESS_2,
};
use crate::paymaster::Error;

#[tokio::test(flavor = "multi_thread")]
async fn test_cartridge_outside_execution_when_caller_is_not_a_controller() {
    let mut server = setup_cartridge_server().await;
    let (node, paymaster, _) = setup(&server.url(), None, None).await;

    deploy_vrf_provider(&node, paymaster.paymaster_address).await;

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

    deploy_vrf_provider(&node, paymaster.paymaster_address).await;

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
    let (node, paymaster, _) = setup(&server.url(), None, None).await;

    deploy_vrf_provider(&node, paymaster.paymaster_address).await;

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

    // use a salt source
    let salt = felt!("0xdeadbeef");
    let outside_execution = craft_valid_outside_execution_v2(
        ContractAddress::from(CONTROLLER_ADDRESS_2),
        vrf_address,
        fake_calls_count,
        None,
        Some(vec![CONTROLLER_ADDRESS_2, Felt::ONE, salt]),
    );

    let expected_seed =
        Poseidon::hash_array(&[salt, CONTROLLER_ADDRESS_2, paymaster.chain_id.id()]);
    let expected_proof = paymaster.vrf_ctx.stark_vrf(expected_seed).unwrap();

    let res = paymaster
        .handle_add_outside_execution(
            ContractAddress::from(CONTROLLER_ADDRESS_2),
            outside_execution,
            vec![],
        )
        .await;

    assert!(res.is_ok());
    assert_crafted_outside_execution(
        &res.unwrap().unwrap().0,
        ContractAddress::from(CONTROLLER_ADDRESS_2),
        vrf_address,
        Some(expected_seed),
        Some(expected_proof),
        selector!("execute_from_outside_v2"),
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
        .nth(2)
        .expect("must have at least one account");

    // deploy the VRF provider
    deploy_vrf_provider(&node, paymaster.paymaster_address).await;

    let mocks =
        setup_mocks(&mut server, &[(&controller_address.to_hex_string(), true, true)]).await;

    let fake_calls_count = 3;

    // Set a non-zero nonce for the controller which calls the VRF provider
    let controller_nonce = Felt::TWO;
    let key = Pedersen::hash(&selector!("VrfProvider_nonces"), &controller_address);

    let provider = paymaster.starknet_api.storage().provider_mut();
    provider.set_storage(vrf_address, key, controller_nonce).expect("failed to set storage");
    provider.commit().expect("failed to commit");

    // use a nonce source
    let outside_execution = craft_valid_outside_execution_v3(
        *controller_address,
        vrf_address,
        fake_calls_count,
        None,
        Some(vec![(*controller_address).into(), Felt::ZERO, (*controller_address).into()]),
    );

    let expected_seed = Poseidon::hash_array(&[
        controller_nonce,
        (*controller_address).into(),
        paymaster.chain_id.id(),
    ]);

    let expected_proof = paymaster.vrf_ctx.stark_vrf(expected_seed).unwrap();

    let res = paymaster
        .handle_add_outside_execution(*controller_address, outside_execution, vec![])
        .await;

    assert!(res.is_ok());
    assert_crafted_outside_execution(
        &res.unwrap().unwrap().0,
        *controller_address,
        vrf_address,
        Some(expected_seed),
        Some(expected_proof),
        selector!("execute_from_outside_v3"),
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
async fn test_cartridge_outside_execution_when_wrong_paymaster_pkey() {
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
