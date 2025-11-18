use std::path::PathBuf;
use std::sync::Arc;

use katana_core::service::block_producer::BlockProducer;
use katana_executor::implementation::blockifier::BlockifierFactory;
use katana_executor::ExecutionFlags;
use katana_gas_price_oracle::GasPriceOracle;
use katana_genesis::constant::DEFAULT_UDC_ADDRESS;
use katana_pool::ordering::FiFo;
use katana_pool::TxPool;
use katana_primitives::chain::ChainId;
use katana_primitives::contract::ContractAddress;
use katana_primitives::da::DataAvailabilityMode;
use katana_primitives::fee::{
    AllResourceBoundsMapping, ResourceBounds, ResourceBoundsMapping, Tip,
};
use katana_primitives::transaction::{ExecutableTx, InvokeTx};
use katana_provider::BlockchainProvider;
use katana_rpc_client::starknet::Client as StarknetClient;
use katana_rpc_server::starknet::{StarknetApi, StarknetApiConfig};
use katana_rpc_types::broadcasted::{BroadcastedInvokeTx, BroadcastedTx};
use katana_tasks::TaskManager;
use katana_test_utils::prepare_contract_declaration_params;
use katana_utils::node::{test_config, TestNode};
use katana_utils::TxWaiter;
use serde_json::json;
use starknet::accounts::Account;
use starknet::core::types::Call as StarknetCall;
use starknet::core::utils::get_selector_from_name;
use starknet::macros::{felt, selector};
use starknet::signers::SigningKey;
use starknet_crypto::Felt;
use url::Url;

use crate::client::Client;
use crate::paymaster::Paymaster;
use crate::rpc::types::{
    Call, NonceChannel, OutsideExecution, OutsideExecutionV2, OutsideExecutionV3,
};
use crate::vrf::{StarkVrfProof, VrfContext, CARTRIDGE_VRF_CLASS_HASH, CARTRIDGE_VRF_SALT};

pub const DEFAULT_NONCE_CHANNEL: u128 = 10;

pub const CONTROLLER_ADDRESS_1: Felt = felt!("0xb0b");
pub const CONTROLLER_ADDRESS_2: Felt = felt!("0xa11ce");

pub const ALREADY_DEPLOYED_CONTROLLER_ADDRESS: Felt =
    felt!("0x1399f8d212a38c4f3b3080573660a15bd26d15036b7638b63540aaefe135c49");

pub const VRF_PROVIDER_CLASS_PATH: &str =
    "src/paymaster/tests/test_data/cartridge_vrf_VrfProvider.contract_class.json";
pub const VRF_PRIVATE_KEY_FOR_TESTS: Felt = felt!("0xdeadbeef");
pub const VRF_PUBLIC_KEY_X: Felt =
    felt!("0x5eeb3e0d88756352e5b7015667431490b631ea109bb6e31d65bb3bef604c186");
pub const VRF_PUBLIC_KEY_Y: Felt =
    felt!("0x30aab4c6959ff79d796cdebf33aa567e01fd0e757a4e560e2d89141b4de1141");

pub fn default_outside_execution() -> OutsideExecution {
    OutsideExecution::V2(OutsideExecutionV2 {
        caller: ContractAddress::from(Felt::ZERO),
        nonce: Felt::ZERO,
        execute_after: 0,
        execute_before: 0,
        calls: vec![],
    })
}

pub fn craft_valid_outside_execution_calls(
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

pub fn craft_valid_outside_execution_v2(
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

pub fn craft_valid_outside_execution_v3(
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

pub fn build_calldata(address: &str) -> Vec<&str> {
    vec![address, "0x42", address]
}

pub fn build_response(address: &str, is_controller: bool) -> String {
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

pub async fn build_mock(
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
pub async fn setup_cartridge_server() -> mockito::Server {
    mockito::Server::new_with_opts_async(Default::default()).await
}

// setup the mocks for the cartridge server
pub async fn setup_mocks(
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
pub async fn setup(
    cartridge_url: &str,
    paymaster_address: Option<ContractAddress>,
    paymaster_private_key: Option<SigningKey>,
) -> (TestNode, Paymaster<TxPool, BlockProducer<BlockifierFactory>>, ContractAddress) {
    let config = test_config();
    let sequencer = TestNode::new_with_config(config.clone()).await;
    let block_producer = BlockProducer::instant(Arc::clone(&sequencer.backend()));
    let validator = block_producer.validator();

    let task_spawner = TaskManager::current().task_spawner();

    let starknet_api = StarknetApi::new(
        sequencer.backend().chain_spec.clone(),
        BlockchainProvider::new(Box::new(sequencer.backend().blockchain.provider().clone())),
        TxPool::new(validator.clone(), FiFo::new()),
        task_spawner,
        block_producer.clone(),
        GasPriceOracle::create_for_testing(),
        StarknetApiConfig {
            max_call_gas: config.rpc.max_call_gas,
            max_proof_keys: config.rpc.max_proof_keys,
            max_event_page_size: config.rpc.max_event_page_size,
            max_concurrent_estimate_fee_requests: config.rpc.max_concurrent_estimate_fee_requests,
            simulation_flags: ExecutionFlags::default(),
            versioned_constant_overrides: None,
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

pub async fn deploy_vrf_provider(node: &TestNode, paymaster_address: ContractAddress) {
    let account = node.account();

    let url = Url::parse(&format!("http://{}", node.rpc_addr())).expect("failed to parse url");
    let starknet_client = StarknetClient::new(url);

    let path = PathBuf::from(VRF_PROVIDER_CLASS_PATH);

    let (contract, compiled_class_hash) = prepare_contract_declaration_params(&path)
        .expect("failed to prepare VRF provider contract");

    let res = account
        .declare_v3(contract.into(), compiled_class_hash)
        .send()
        .await
        .expect("failed to send declare tx");

    katana_utils::TxWaiter::new(res.transaction_hash, &starknet_client)
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

    TxWaiter::new(tx.transaction_hash, &starknet_client).await.expect("failed to wait on tx");
}

/// Just build a fake invoke transaction.
pub fn invoke_tx(sender_address: ContractAddress) -> BroadcastedTx {
    BroadcastedTx::Invoke(BroadcastedInvokeTx {
        sender_address,
        calldata: vec![],
        signature: vec![],
        nonce: Felt::ZERO,
        paymaster_data: vec![],
        tip: Tip::new(0),
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

pub fn assert_mocks(mocks: &[mockito::Mock]) {
    mocks.iter().for_each(|mock| mock.assert());
}

pub fn assert_outside_execution_v2(
    new_outside_execution: &OutsideExecution,
    fake_calls_count: usize,
    expected_nonce: Felt,
    vrf_address: ContractAddress,
    paymaster_address: ContractAddress,
    seed: Option<Felt>,
    proof: Option<StarkVrfProof>,
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

        if let Some(seed) = seed {
            assert_eq!(v2.calls[0].calldata[0], seed);
        }

        if let Some(proof) = proof {
            assert_eq!(v2.calls[0].calldata[1], Felt::from_hex_unchecked(&proof.gamma_x));
            assert_eq!(v2.calls[0].calldata[2], Felt::from_hex_unchecked(&proof.gamma_y));
            assert_eq!(v2.calls[0].calldata[3], Felt::from_hex_unchecked(&proof.c));
            assert_eq!(v2.calls[0].calldata[4], Felt::from_hex_unchecked(&proof.s));
            assert_eq!(v2.calls[0].calldata[5], Felt::from_hex_unchecked(&proof.sqrt_ratio));
        }

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

        if let Some(seed) = seed {
            assert_eq!(v2.calls[last_call_index].calldata[0], seed);
        }
    } else {
        panic!("expected OutsideExecution::V2");
    }
}

pub fn assert_outside_execution_v3(
    new_outside_execution: &OutsideExecution,
    fake_calls_count: usize,
    expected_nonce: Felt,
    vrf_address: ContractAddress,
    paymaster_address: ContractAddress,
    seed: Option<Felt>,
    proof: Option<StarkVrfProof>,
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

        if let Some(seed) = seed {
            assert_eq!(v3.calls[0].calldata[0], seed);
        }

        if let Some(proof) = proof {
            assert_eq!(v3.calls[0].calldata[1], Felt::from_hex_unchecked(&proof.gamma_x));
            assert_eq!(v3.calls[0].calldata[2], Felt::from_hex_unchecked(&proof.gamma_y));
            assert_eq!(v3.calls[0].calldata[3], Felt::from_hex_unchecked(&proof.c));
            assert_eq!(v3.calls[0].calldata[4], Felt::from_hex_unchecked(&proof.s));
            assert_eq!(v3.calls[0].calldata[5], Felt::from_hex_unchecked(&proof.sqrt_ratio));
        }

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

        if let Some(seed) = seed {
            assert_eq!(v3.calls[last_call_index].calldata[0], seed);
        }
    } else {
        panic!("expected OutsideExecution::V3");
    }
}

pub fn assert_tx(tx: &BroadcastedTx, address: &str) {
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

pub fn assert_controller_deploy_tx(
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

pub fn assert_vrf_provider_deploy_tx(
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
