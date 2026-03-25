#![cfg(feature = "cartridge")]

//! Integration tests for the Cartridge VRF flow.
//!
//! Tests that when `cartridge_addExecuteOutsideTransaction` includes a `request_random`
//! call, the CartridgeApi delegates to the VRF service and forwards the modified
//! execution to the paymaster.
//!
//! Uses a mock VRF server and mock paymaster to validate the wiring without
//! requiring external binaries.

use std::sync::{Arc, Mutex};

use jsonrpsee::core::client::ClientT;
use katana_primitives::Felt;
use katana_utils::node::test_config;
use katana_utils::TestNode;
use serde_json::json;
use starknet::macros::selector;

mod common;

// ---------------------------------------------------------------------------
// Mock Cartridge API (reused pattern)
// ---------------------------------------------------------------------------

async fn start_mock_cartridge_api() -> url::Url {
    use axum::routing::post;
    use axum::Router;
    use tokio::net::TcpListener;

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

// ---------------------------------------------------------------------------
// Mock Paymaster
// ---------------------------------------------------------------------------

async fn start_mock_paymaster() -> (url::Url, MockPaymasterState) {
    use jsonrpsee::core::{async_trait, RpcResult};
    use jsonrpsee::server::ServerBuilder;
    use katana_rpc_api::paymaster::PaymasterApiServer;
    use paymaster_rpc::{
        BuildTransactionRequest, BuildTransactionResponse, ExecuteRawRequest, ExecuteRequest,
        ExecuteResponse, TokenPrice,
    };

    // We need the paymaster to record whether it was called and with what user_address.
    // jsonrpsee server handlers don't have shared state easily, so we use a global.
    static PAYMASTER_CALLS: Mutex<Vec<serde_json::Value>> = Mutex::new(Vec::new());
    // Clear from any previous run.
    PAYMASTER_CALLS.lock().unwrap().clear();

    struct MockPaymaster;

    #[async_trait]
    impl PaymasterApiServer for MockPaymaster {
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
        ) -> RpcResult<paymaster_rpc::ExecuteRawResponse> {
            let serialized = serde_json::to_value(&req).unwrap();
            PAYMASTER_CALLS.lock().unwrap().push(serialized);

            let response: paymaster_rpc::ExecuteRawResponse =
                serde_json::from_str(r#"{"transaction_hash": "0xcafe", "tracking_id": "0x0"}"#)
                    .unwrap();
            Ok(response)
        }
        async fn get_supported_tokens(&self) -> RpcResult<Vec<TokenPrice>> {
            Ok(vec![])
        }
    }

    let server = ServerBuilder::default().build("127.0.0.1:0").await.unwrap();
    let addr = server.local_addr().unwrap();
    let handle = server.start(MockPaymaster.into_rpc());
    std::mem::forget(handle);

    let url = url::Url::parse(&format!("http://{addr}")).unwrap();
    let state = MockPaymasterState { calls: &PAYMASTER_CALLS };
    (url, state)
}

struct MockPaymasterState {
    calls: &'static Mutex<Vec<serde_json::Value>>,
}

impl MockPaymasterState {
    fn take_calls(&self) -> Vec<serde_json::Value> {
        std::mem::take(&mut *self.calls.lock().unwrap())
    }
}

// ---------------------------------------------------------------------------
// Mock VRF Server
// ---------------------------------------------------------------------------

/// Starts a mock VRF server that responds to `GET /info` and `POST /outside_execution`.
///
/// The `/outside_execution` handler returns a modified `SignedOutsideExecution`
/// where the address is changed to the VRF account (simulating the VRF server
/// wrapping the execution with proof submission).
async fn start_mock_vrf_server(
    vrf_account_address: katana_primitives::ContractAddress,
) -> (url::Url, MockVrfState) {
    use axum::routing::{get, post};
    use axum::Router;
    use tokio::net::TcpListener;

    let state =
        MockVrfState { vrf_account_address, received_requests: Arc::new(Mutex::new(Vec::new())) };

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
    use axum::response::IntoResponse;
    // Return dummy VRF public key info.
    axum::Json(json!({
        "public_key_x": "0x66da5d53168d591c55d4c05f3681663ac51bcdccd5ca09e366b71b0c40ccff4",
        "public_key_y": "0x6d3eb29920bf55195e5ec76f69e247c0942c7ef85f6640896c058ec75ca2232"
    }))
    .into_response()
}

async fn vrf_outside_execution_handler(
    axum::extract::State(state): axum::extract::State<MockVrfState>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    state.received_requests.lock().unwrap().push(body.clone());

    // The VRF server returns a modified SignedOutsideExecution where:
    // - address is the VRF account (outer execution is on the VRF account)
    // - outside_execution calls include submit_random + execute_from_outside
    // - signature is from the VRF account
    //
    // For this mock we return a minimal valid response that the CartridgeApi
    // will convert to a Call and forward to the paymaster.
    let vrf_addr = format!("{:#x}", Felt::from(state.vrf_account_address));

    let response = json!({
        "result": {
            "address": vrf_addr,
            "outside_execution": {
                "V2": {
                    "caller": "0x414e595f43414c4c4552",
                    "nonce": "0x99",
                    "execute_after": "0x0",
                    "execute_before": "0xffffffffffffffff",
                    "calls": [{
                        "to": vrf_addr,
                        "selector": format!("{:#x}", selector!("submit_random")),
                        "calldata": ["0x1", "0x2"]
                    }]
                }
            },
            "signature": ["0xaa", "0xbb"]
        }
    });

    axum::Json(response).into_response()
}

#[derive(Clone)]
struct MockVrfState {
    vrf_account_address: katana_primitives::ContractAddress,
    received_requests: Arc<Mutex<Vec<serde_json::Value>>>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build execute_outside params that include a `request_random` call targeting
/// the VRF account, followed by a game call — the pattern that triggers VRF delegation.
fn make_vrf_execute_outside_params(
    player_address: &str,
    vrf_account_address: &str,
) -> Vec<serde_json::Value> {
    vec![
        json!(player_address),
        json!({
            "caller": "0x414e595f43414c4c4552",
            "nonce": "0x1",
            "execute_after": "0x0",
            "execute_before": "0xffffffffffffffff",
            "calls": [
                {
                    "to": vrf_account_address,
                    "selector": format!("{:#x}", selector!("request_random")),
                    "calldata": ["0x1", "0x2"]
                },
                {
                    "to": "0xaaa",
                    "selector": "0xbbb",
                    "calldata": []
                }
            ]
        }),
        json!(["0x0", "0x0"]),
        json!(null),
    ]
}

/// Build execute_outside params with request_random but targeting the WRONG address.
fn make_vrf_wrong_target_params(player_address: &str) -> Vec<serde_json::Value> {
    vec![
        json!(player_address),
        json!({
            "caller": "0x414e595f43414c4c4552",
            "nonce": "0x1",
            "execute_after": "0x0",
            "execute_before": "0xffffffffffffffff",
            "calls": [
                {
                    "to": "0xdead",
                    "selector": format!("{:#x}", selector!("request_random")),
                    "calldata": ["0x1"]
                },
                {
                    "to": "0xaaa",
                    "selector": "0xbbb",
                    "calldata": []
                }
            ]
        }),
        json!(["0x0", "0x0"]),
        json!(null),
    ]
}

/// Build execute_outside params where request_random is the LAST call (no follow-up).
fn make_vrf_no_followup_params(
    player_address: &str,
    vrf_account_address: &str,
) -> Vec<serde_json::Value> {
    vec![
        json!(player_address),
        json!({
            "caller": "0x414e595f43414c4c4552",
            "nonce": "0x1",
            "execute_after": "0x0",
            "execute_before": "0xffffffffffffffff",
            "calls": [{
                "to": vrf_account_address,
                "selector": format!("{:#x}", selector!("request_random")),
                "calldata": ["0x1"]
            }]
        }),
        json!(["0x0", "0x0"]),
        json!(null),
    ]
}

fn cartridge_vrf_test_config(
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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
#[tokio::test(flavor = "multi_thread")]
async fn vrf_delegation_flow() {
    let vrf_account_address = katana_primitives::ContractAddress::from(Felt::from(0xBAAD_u64));
    let vrf_account_hex = format!("{:#x}", Felt::from(vrf_account_address));
    // Use the deployer (genesis account) as the "player" so it's already deployed
    // and the controller deployment middleware won't interfere.

    let cartridge_api_url = start_mock_cartridge_api().await;
    let (paymaster_url, paymaster_state) = start_mock_paymaster().await;
    let (vrf_url, vrf_state) = start_mock_vrf_server(vrf_account_address).await;

    let config =
        cartridge_vrf_test_config(cartridge_api_url, paymaster_url, vrf_url, vrf_account_address);
    let player_address = {
        let (addr, _) = config.chain.genesis().accounts().next().unwrap();
        format!("{:#x}", Felt::from(*addr))
    };

    let node = TestNode::new_with_config(config).await;
    let client = node.rpc_http_client();

    // -----------------------------------------------------------------------
    // Case 1: Valid VRF flow — request_random targeting VRF account
    // -----------------------------------------------------------------------
    {
        let params = make_vrf_execute_outside_params(&player_address, &vrf_account_hex);
        let _: serde_json::Value = client
            .request("cartridge_addExecuteOutsideTransaction", params)
            .await
            .expect("VRF execute outside should succeed");

        // VRF server should have received the outside execution request.
        let vrf_requests = vrf_state.received_requests.lock().unwrap();
        assert_eq!(vrf_requests.len(), 1, "VRF server should have been called once");

        // Paymaster should have received the modified request with VRF account as user.
        let paymaster_calls = paymaster_state.take_calls();
        assert_eq!(paymaster_calls.len(), 1, "paymaster should have been called once");

        // The paymaster request should use the VRF account address (from the VRF server response).
        let pm_request = &paymaster_calls[0];
        let user_address = pm_request
            .pointer("/transaction/invoke/user_address")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert_eq!(
            user_address, vrf_account_hex,
            "paymaster request should use VRF account as user_address"
        );
    }

    // -----------------------------------------------------------------------
    // Case 2: request_random targeting wrong address → VrfInvalidTarget
    // -----------------------------------------------------------------------
    {
        let params = make_vrf_wrong_target_params(&player_address);
        let result: Result<serde_json::Value, _> =
            client.request("cartridge_addExecuteOutsideTransaction", params).await;

        assert!(result.is_err(), "should fail with VrfInvalidTarget");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("request_random") || err.contains("VRF"),
            "error should mention VRF target: {err}"
        );
    }

    // -----------------------------------------------------------------------
    // Case 3: request_random as last call (no follow-up) → VrfMissingFollowUpCall
    // -----------------------------------------------------------------------
    {
        let params = make_vrf_no_followup_params(&player_address, &vrf_account_hex);
        let result: Result<serde_json::Value, _> =
            client.request("cartridge_addExecuteOutsideTransaction", params).await;

        assert!(result.is_err(), "should fail with VrfMissingFollowUpCall");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("followed by another call") || err.contains("follow"),
            "error should mention missing follow-up call: {err}"
        );
    }

    // -----------------------------------------------------------------------
    // Case 4: No request_random → normal flow, VRF not involved
    // -----------------------------------------------------------------------
    {
        let prev_vrf_count = vrf_state.received_requests.lock().unwrap().len();

        // Use plain params without request_random.
        let params = vec![
            json!(player_address),
            json!({
                "caller": "0x414e595f43414c4c4552",
                "nonce": "0x2",
                "execute_after": "0x0",
                "execute_before": "0xffffffffffffffff",
                "calls": [{
                    "to": "0x1",
                    "selector": "0x2",
                    "calldata": ["0x3"]
                }]
            }),
            json!(["0x0", "0x0"]),
            json!(null),
        ];

        let _: serde_json::Value = client
            .request("cartridge_addExecuteOutsideTransaction", params)
            .await
            .expect("non-VRF execute outside should succeed");

        // VRF server should NOT have been called.
        let vrf_requests = vrf_state.received_requests.lock().unwrap();
        assert_eq!(
            vrf_requests.len(),
            prev_vrf_count,
            "VRF server should not be called for non-VRF requests"
        );

        // Paymaster should have been called with the player address (not VRF account).
        let paymaster_calls = paymaster_state.take_calls();
        assert_eq!(paymaster_calls.len(), 1, "paymaster should have been called");

        let pm_request = &paymaster_calls[0];
        let user_address = pm_request
            .pointer("/transaction/invoke/user_address")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert_eq!(
            user_address, player_address,
            "paymaster request should use player address (not VRF account)"
        );
    }
}
