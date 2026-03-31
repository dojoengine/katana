#![cfg(feature = "cartridge")]

//! Integration tests for the Cartridge VRF flow.
//!
//! Tests that when `cartridge_addExecuteOutsideTransaction` includes a `request_random`
//! call, the CartridgeApi delegates to the VRF service and forwards the modified
//! execution to the paymaster.
//!
//! Uses a mock VRF server and mock paymaster to validate the wiring without
//! requiring external binaries.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use cartridge::vrf::{RequestContext, SignedOutsideExecution, VrfOutsideExecution};
use jsonrpsee::core::{async_trait, RpcResult};
use jsonrpsee::server::ServerBuilder;
use katana_primitives::execution::Call;
use katana_primitives::{address, felt, ContractAddress, Felt};
use katana_rpc_api::cartridge::CartridgeApiClient;
use katana_rpc_api::paymaster::PaymasterApiServer;
use katana_rpc_types::{OutsideExecution, OutsideExecutionV2};
use katana_utils::node::test_config;
use katana_utils::TestNode;
use parking_lot::Mutex;
use paymaster_rpc::{
    BuildTransactionRequest, BuildTransactionResponse, ExecuteRawRequest, ExecuteRawResponse,
    ExecuteRawTransactionParameters, ExecuteRequest, ExecuteResponse, RawInvokeParameters,
    TokenPrice,
};
use serde::Deserialize;
use serde_json::json;
use starknet::accounts::Account;
use starknet::macros::selector;
use tokio::net::TcpListener;
use url::Url;

mod common;

/// Test the VRF delegation flow in CartridgeApi.
///
/// When an outside execution includes `request_random` targeting the configured
/// VRF account, the CartridgeApi should:
/// 1. Delegate to the VRF server
/// 2. Use the VRF server's response (modified execution) to build the paymaster request
/// 3. Forward to paymaster with the VRF account as user_address
///
/// Also tests error cases:
/// - request_random targeting wrong address → VrfInvalidTarget error
/// - request_random as last call (no follow-up) → VrfMissingFollowUpCall error
/// - No request_random → normal flow (no VRF delegation)
#[tokio::test]
async fn valid_vrf_delegation_flow() {
    let vrf_address = address!("0xBAAD");

    let setup = setup_test(vrf_address).await;
    let TestSetup { node, paymaster_svc_state, vrf_svc_state } = setup;

    let player_address = node.account().address();

    let player_signature = vec![felt!("0x0"), felt!("0x0")];
    let outside_execution = OutsideExecution::V2(OutsideExecutionV2 {
        caller: address!("0x414e595f43414c4c4552"),
        nonce: felt!("0x1"),
        execute_after: 0,
        execute_before: 0xffffffffffffffff,
        calls: vec![
            Call {
                contract_address: vrf_address,
                entry_point_selector: selector!("request_random"),
                calldata: vec![felt!("0x1"), felt!("0x2")],
            },
            Call {
                contract_address: address!("0xaaa"),
                entry_point_selector: felt!("0xbbb"),
                calldata: Vec::new(),
            },
        ],
    });

    let _ = node
        .rpc_http_client()
        .add_execute_outside_transaction(
            player_address.into(),
            outside_execution,
            player_signature,
            None,
        )
        .await
        .expect("VRF execute outside should succeed");

    // VRF server should have received the outside execution request.
    let vrf_requests = vrf_svc_state.received_requests.lock();
    assert_eq!(vrf_requests.len(), 1, "VRF server should have been called once");
    let vrf_request = vrf_requests.get(&player_address.into());
    assert!(vrf_request.is_some());

    // Paymaster should have received the modified request with VRF account as user.
    let requests = paymaster_svc_state.execute_raw_tx_requests.lock();
    let calls = requests.get(&vrf_address).unwrap();
    assert_eq!(calls.len(), 1, "paymaster should have been called once");
    assert_eq!(
        calls[0].user_address,
        vrf_address.into(),
        "final request must use VRF account as user_address"
    );
}

#[tokio::test]
async fn request_random_with_no_follow_up_call() {
    let vrf_account_address = address!("0xBAAD");

    let setup = setup_test(vrf_account_address).await;
    let TestSetup { node, paymaster_svc_state, vrf_svc_state } = setup;

    let sender: ContractAddress = node.account().address().into();
    let sender_signature = vec![felt!("0x0"), felt!("0x0")];

    let outside_execution = OutsideExecution::V2(OutsideExecutionV2 {
        caller: address!("0x414e595f43414c4c4552"),
        nonce: felt!("0x1"),
        execute_after: 0,
        execute_before: 0xffffffffffffffff,
        calls: vec![Call {
            contract_address: vrf_account_address,
            entry_point_selector: selector!("request_random"),
            calldata: vec![felt!("0x1")],
        }],
    });

    let err = node
        .rpc_http_client()
        .add_execute_outside_transaction(sender, outside_execution, sender_signature, None)
        .await
        .expect_err("should fail due to missing follow up call");

    assert!(err.to_string().contains("request_random call must be followed by another call"));

    let calls = paymaster_svc_state.execute_raw_tx_requests.lock();
    assert!(calls.get(&sender).is_none(), "paymaster shouldnt be called on no follow up call");

    let calls = vrf_svc_state.received_requests.lock();
    assert!(calls.get(&sender).is_none(), "vrf shouldnt be called on no follow up call");
}

#[tokio::test]
async fn request_random_targeting_wrong_vrf_address() {
    let vrf_account_address = address!("0xBAAD");

    let setup = setup_test(vrf_account_address).await;
    let TestSetup { node, paymaster_svc_state, vrf_svc_state } = setup;

    let sender: ContractAddress = node.account().address().into();
    let sender_signature = vec![felt!("0x0"), felt!("0x0")];

    let outside_execution = OutsideExecution::V2(OutsideExecutionV2 {
        caller: address!("0x414e595f43414c4c4552"),
        nonce: felt!("0x1"),
        execute_after: 0x0,
        execute_before: 0xffffffffffffffff,
        calls: vec![
            Call {
                contract_address: address!("0xdead"),
                entry_point_selector: selector!("request_random"),
                calldata: vec![felt!("0x1")],
            },
            Call {
                contract_address: address!("0xaaa"),
                entry_point_selector: felt!("0xbbb"),
                calldata: Vec::new(),
            },
        ],
    });

    let err = node
        .rpc_http_client()
        .add_execute_outside_transaction(sender, outside_execution, sender_signature, None)
        .await
        .expect_err("should fail due to invalid vrf address");

    assert!(err.to_string().contains("request_random call must target the VRF account"));

    let calls = paymaster_svc_state.execute_raw_tx_requests.lock();
    assert!(calls.get(&sender).is_none(), "paymaster shouldnt be called on no follow up call");

    let calls = vrf_svc_state.received_requests.lock();
    assert!(calls.get(&sender).is_none(), "vrf shouldnt be called on no follow up call");
}

#[tokio::test]
async fn normal_flow_when_no_request_random_call() {
    let vrf_account_address = address!("0xBAAD");

    let setup = setup_test(vrf_account_address).await;
    let TestSetup { node, paymaster_svc_state, vrf_svc_state } = setup;

    let sender: ContractAddress = node.account().address().into();
    let sender_signature = vec![felt!("0x0"), felt!("0x0")];

    let outside_execution = OutsideExecution::V2(OutsideExecutionV2 {
        caller: address!("0x414e595f43414c4c4552"),
        nonce: felt!("0x1"),
        execute_after: 0,
        execute_before: 0xffffffffffffffff,
        calls: vec![Call {
            contract_address: address!("0x1"),
            entry_point_selector: felt!("0x2"),
            calldata: vec![felt!("0x3")],
        }],
    });

    let _ = node
        .rpc_http_client()
        .add_execute_outside_transaction(sender, outside_execution, sender_signature, None)
        .await
        .expect("should succeed in normal flow");

    let calls = vrf_svc_state.received_requests.lock();
    assert!(calls.get(&sender).is_none(), "vrf shouldnt be called on no follow up call");

    let calls = paymaster_svc_state.execute_raw_tx_requests.lock();
    let calls = calls.get(&sender).expect("tx should be forwarded to paymaster");
    assert_eq!(calls.len(), 1, "paymaster should only be called once");

    let call = &calls[0];
    assert_eq!(call.user_address, sender.into());
}

struct TestSetup {
    paymaster_svc_state: MockPaymasterState,
    vrf_svc_state: MockVrfState,
    node: TestNode,
}

async fn setup_test(vrf_account_address: ContractAddress) -> TestSetup {
    let cartridge_api_url = start_mock_cartridge_api().await;
    let (paymaster_url, paymaster_svc_state) = start_mock_paymaster().await;
    let (vrf_url, vrf_svc_state) = start_mock_vrf_server(vrf_account_address).await;

    let cfg = create_node_config(cartridge_api_url, paymaster_url, vrf_url, vrf_account_address);
    let node = TestNode::new_with_config(cfg).await;

    TestSetup { paymaster_svc_state, vrf_svc_state, node }
}

async fn start_mock_cartridge_api() -> url::Url {
    async fn handler(axum::Json(_body): axum::Json<serde_json::Value>) -> axum::response::Response {
        use axum::response::IntoResponse;
        "Address not found".into_response()
    }

    let app = Router::new().route("/accounts/calldata", post(handler));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    url::Url::parse(&format!("http://{addr}")).unwrap()
}

#[derive(Default, Clone)]
struct MockPaymasterState {
    // track execute_raw_transaction requests
    execute_raw_tx_requests: Arc<Mutex<HashMap<ContractAddress, Vec<RawInvokeParameters>>>>,
}

#[async_trait]
impl PaymasterApiServer for MockPaymasterState {
    async fn health(&self) -> RpcResult<bool> {
        Ok(true)
    }

    async fn is_available(&self) -> RpcResult<bool> {
        Ok(true)
    }

    async fn build_transaction(
        &self,
        _req: BuildTransactionRequest,
    ) -> RpcResult<BuildTransactionResponse> {
        unimplemented!()
    }

    async fn execute_transaction(&self, _req: ExecuteRequest) -> RpcResult<ExecuteResponse> {
        unimplemented!()
    }

    async fn execute_raw_transaction(
        &self,
        req: ExecuteRawRequest,
    ) -> RpcResult<ExecuteRawResponse> {
        match req.transaction {
            ExecuteRawTransactionParameters::RawInvoke { invoke } => {
                let sender_address = invoke.user_address;
                self.execute_raw_tx_requests
                    .lock()
                    .entry(sender_address.into())
                    .or_default()
                    .push(invoke);
            }
        }

        Ok(ExecuteRawResponse { transaction_hash: felt!("0xcafe"), tracking_id: Felt::ZERO })
    }

    async fn get_supported_tokens(&self) -> RpcResult<Vec<TokenPrice>> {
        Ok(vec![])
    }
}

async fn start_mock_paymaster() -> (url::Url, MockPaymasterState) {
    let paymaster_state = MockPaymasterState::default();

    let server = ServerBuilder::default().build("127.0.0.1:0").await.unwrap();
    let addr = server.local_addr().unwrap();
    let handle = server.start(paymaster_state.clone().into_rpc());
    std::mem::forget(handle);

    let url = url::Url::parse(&format!("http://{addr}")).unwrap();
    (url, paymaster_state)
}

async fn start_mock_vrf_server(vrf_account_address: ContractAddress) -> (Url, MockVrfState) {
    let state = MockVrfState { vrf_account_address, received_requests: Default::default() };

    let app = Router::new()
        .route("/info", get(vrf_info_handler))
        .route("/outside_execution", post(vrf_outside_execution_handler))
        .with_state(state.clone());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let url = url::Url::parse(&format!("http://{addr}")).unwrap();
    (url, state)
}

async fn vrf_info_handler() -> axum::response::Response {
    // Return dummy VRF public key info.
    Json(json!({
        "public_key_x": "0x66da5d53168d591c55d4c05f3681663ac51bcdccd5ca09e366b71b0c40ccff4",
        "public_key_y": "0x6d3eb29920bf55195e5ec76f69e247c0942c7ef85f6640896c058ec75ca2232"
    }))
    .into_response()
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
struct OutsideExecutionRequest {
    request: SignedOutsideExecution,
    context: RequestContext,
}

async fn vrf_outside_execution_handler(
    State(state): State<MockVrfState>,
    Json(body): Json<OutsideExecutionRequest>,
) -> Response {
    let sender = body.request.address;
    state.received_requests.lock().insert(sender, body.clone());

    // The VRF server returns a modified SignedOutsideExecution where:
    // - address is the VRF account (outer execution is on the VRF account)
    // - outside_execution calls include submit_random + execute_from_outside
    // - signature is from the VRF account
    //
    // For this mock we return a minimal valid response that the CartridgeApi
    // will convert to a Call and forward to the paymaster.
    let updated_outside_execution = SignedOutsideExecution {
        address: state.vrf_account_address,
        outside_execution: VrfOutsideExecution::V2(OutsideExecutionV2 {
            caller: address!("0x414e595f43414c4c4552"),
            nonce: felt!("0x99"),
            execute_after: 0x0,
            execute_before: 0xffffffffffffffff,
            calls: vec![Call {
                contract_address: state.vrf_account_address,
                entry_point_selector: selector!("submit_random"),
                calldata: vec![felt!("0x1"), felt!("0x2")],
            }],
        }),
        signature: vec![felt!("0xaa"), felt!("0xbb")],
    };

    let response = json!({
        "result": updated_outside_execution
    });

    Json(response).into_response()
}

#[derive(Clone)]
struct MockVrfState {
    vrf_account_address: ContractAddress,
    received_requests: Arc<Mutex<HashMap<ContractAddress, OutsideExecutionRequest>>>,
}

fn create_node_config(
    cartridge_api_url: url::Url,
    paymaster_url: url::Url,
    vrf_url: url::Url,
    vrf_account_address: katana_primitives::ContractAddress,
) -> katana_sequencer_node::config::Config {
    use katana_sequencer_node::config::paymaster::{
        CartridgeApiConfig, PaymasterConfig, VrfConfig,
    };

    let mut config = test_config();
    config.sequencing.no_mining = true;

    let (deployer_address, deployer_account) =
        config.chain.genesis().accounts().next().expect("must have genesis accounts");
    let deployer_private_key = deployer_account.private_key().expect("must have private key");

    config.paymaster = Some(PaymasterConfig {
        url: paymaster_url,
        api_key: None,
        cartridge_api: Some(CartridgeApiConfig {
            cartridge_api_url,
            controller_deployer_address: *deployer_address,
            controller_deployer_private_key: deployer_private_key,
            vrf: Some(VrfConfig { url: vrf_url, vrf_account: vrf_account_address }),
        }),
    });

    config
}
