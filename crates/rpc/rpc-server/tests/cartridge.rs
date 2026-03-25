#![cfg(feature = "cartridge")]

//! Integration tests for the Cartridge RPC API and Controller deployment middleware.
//!
//! These tests start a real Katana node (with no-mining) and verify that the
//! `cartridge_addExecuteOutsideTransaction` flow correctly deploys undeployed
//! Controller accounts into the transaction pool via the middleware.
//!
//! NOTE: Only one `TestNode` can be created per test binary because the global
//! class cache (`OnceLock`) can only be initialized once. All tests share a
//! single node. Use `cargo nextest` to run separate test binaries in parallel.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use jsonrpsee::core::client::ClientT;
use katana_primitives::Felt;
use katana_rpc_types::txpool::TxPoolStatus;
use katana_utils::node::test_config;
use katana_utils::TestNode;
use serde_json::json;

mod common;

// ---------------------------------------------------------------------------
// Mock Cartridge API
// ---------------------------------------------------------------------------

async fn start_mock_cartridge_api(
    responses: HashMap<String, serde_json::Value>,
) -> (url::Url, MockCartridgeApiState) {
    use axum::routing::post;
    use axum::Router;
    use tokio::net::TcpListener;

    let state = MockCartridgeApiState {
        responses: Arc::new(responses),
        received_requests: Arc::new(Mutex::new(Vec::new())),
    };

    let app = Router::new()
        .route("/accounts/calldata", post(mock_cartridge_handler))
        .with_state(state.clone());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let url = url::Url::parse(&format!("http://{addr}")).unwrap();
    (url, state)
}

async fn mock_cartridge_handler(
    axum::extract::State(state): axum::extract::State<MockCartridgeApiState>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    state.received_requests.lock().unwrap().push(body.clone());

    let address = body.get("address").and_then(|v| v.as_str()).unwrap_or("");

    if let Some(response) = state.responses.get(address) {
        axum::Json(response.clone()).into_response()
    } else {
        "Address not found".into_response()
    }
}

#[derive(Clone)]
struct MockCartridgeApiState {
    responses: Arc<HashMap<String, serde_json::Value>>,
    received_requests: Arc<Mutex<Vec<serde_json::Value>>>,
}

// ---------------------------------------------------------------------------
// Mock Paymaster
// ---------------------------------------------------------------------------

async fn start_mock_paymaster() -> url::Url {
    use jsonrpsee::core::{async_trait, RpcResult};
    use jsonrpsee::server::ServerBuilder;
    use katana_rpc_api::paymaster::PaymasterApiServer;
    use paymaster_rpc::{
        BuildTransactionRequest, BuildTransactionResponse, ExecuteRawRequest, ExecuteRequest,
        ExecuteResponse, TokenPrice,
    };

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
            _req: ExecuteRawRequest,
        ) -> RpcResult<paymaster_rpc::ExecuteRawResponse> {
            let response: paymaster_rpc::ExecuteRawResponse = serde_json::from_str(
                r#"{"transaction_hash": "0xcafe", "tracking_id": "0x0"}"#,
            )
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

    url::Url::parse(&format!("http://{addr}")).unwrap()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn controller_calldata_response(address: &str) -> serde_json::Value {
    json!({
        "address": address,
        "username": "testuser",
        "calldata": [
            "0x24a9edbfa7082accfceabf6a92d7160086f346d622f28741bf1c651c412c9ab",
            "0x7465737475736572",
            "0x0",
            "0x2",
            "0x1",
            "0x2"
        ]
    })
}

fn make_execute_outside_params(address: &str) -> Vec<serde_json::Value> {
    vec![
        json!(address),
        json!({
            "caller": "0x414e595f43414c4c4552",
            "nonce": "0x1",
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
    ]
}

fn cartridge_test_config(
    cartridge_api_url: url::Url,
    paymaster_url: url::Url,
) -> katana_sequencer_node::config::Config {
    use katana_sequencer_node::config::paymaster::{CartridgeApiConfig, PaymasterConfig};

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
            #[cfg(feature = "vrf")]
            vrf: None,
        }),
    });

    config
}

fn deployer_address(
    config: &katana_sequencer_node::config::Config,
) -> katana_primitives::ContractAddress {
    let (addr, _) = config.chain.genesis().accounts().next().unwrap();
    *addr
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Test the Controller deployment middleware with execute_outside and estimate_fee.
///
/// Uses a single node (due to global class cache constraint) and exercises:
/// 1. Undeployed Controller → deploy tx added to pool
/// 2. Already-deployed address → no deploy tx
/// 3. Non-Controller address → no deploy tx
/// 4. estimate_fee with undeployed Controller → middleware prepends deploy tx
#[tokio::test(flavor = "multi_thread")]
async fn controller_deployment_middleware() {
    let controller_address = "0xdead";
    let non_controller_address = "0xbeef";

    // Register 0xdead as a Controller in the mock Cartridge API.
    let cartridge_responses = HashMap::from_iter([(
        controller_address.to_string(),
        controller_calldata_response(controller_address),
    )]);

    let (cartridge_api_url, mock_api_state) = start_mock_cartridge_api(cartridge_responses).await;
    let paymaster_url = start_mock_paymaster().await;
    let config = cartridge_test_config(cartridge_api_url, paymaster_url);
    let deployer = deployer_address(&config);
    let genesis_address = format!("{:#x}", Felt::from(deployer));

    let node = TestNode::new_with_config(config).await;
    let client = node.rpc_http_client();

    // -----------------------------------------------------------------------
    // Case 1: Undeployed Controller → deploy tx added to pool
    // -----------------------------------------------------------------------
    {
        let params = make_execute_outside_params(controller_address);
        let _: serde_json::Value = client
            .request("cartridge_addExecuteOutsideTransaction", params)
            .await
            .expect("RPC call should succeed");

        // Cartridge API should have been queried.
        let api_requests = mock_api_state.received_requests.lock().unwrap();
        assert_eq!(api_requests.len(), 1, "Cartridge API should have been queried once");

        // Pool should contain 1 deploy tx.
        let status: TxPoolStatus = client
            .request("txpool_status", Vec::<serde_json::Value>::new())
            .await
            .unwrap();
        assert_eq!(status.pending, 1, "pool should contain 1 deploy transaction");

        // Deploy tx should be from the deployer address.
        let content: katana_rpc_types::txpool::TxPoolContent = client
            .request("txpool_contentFrom", vec![json!(deployer)])
            .await
            .unwrap();
        assert_eq!(content.pending.len(), 1, "deploy tx should be from the deployer");
    }

    // -----------------------------------------------------------------------
    // Case 2: Already-deployed address (genesis account) → no extra deploy tx
    // -----------------------------------------------------------------------
    {
        let prev_count = mock_api_state.received_requests.lock().unwrap().len();

        let params = make_execute_outside_params(&genesis_address);
        let _: serde_json::Value = client
            .request("cartridge_addExecuteOutsideTransaction", params)
            .await
            .expect("RPC call should succeed");

        // Cartridge API should NOT have been queried (address is deployed).
        let api_requests = mock_api_state.received_requests.lock().unwrap();
        assert_eq!(
            api_requests.len(),
            prev_count,
            "Cartridge API should not be queried for deployed accounts"
        );

        // Pool should still contain only the 1 deploy tx from case 1.
        let status: TxPoolStatus = client
            .request("txpool_status", Vec::<serde_json::Value>::new())
            .await
            .unwrap();
        assert_eq!(status.pending, 1, "pool should still have only 1 tx");
    }

    // -----------------------------------------------------------------------
    // Case 3: Non-controller undeployed address → no deploy tx
    // -----------------------------------------------------------------------
    {
        let prev_count = mock_api_state.received_requests.lock().unwrap().len();

        let params = make_execute_outside_params(non_controller_address);
        let _: serde_json::Value = client
            .request("cartridge_addExecuteOutsideTransaction", params)
            .await
            .expect("RPC call should succeed");

        // Cartridge API WAS queried (address is undeployed, middleware checks).
        let api_requests = mock_api_state.received_requests.lock().unwrap();
        assert_eq!(
            api_requests.len(),
            prev_count + 1,
            "Cartridge API should be queried for undeployed address"
        );

        // Pool should still have only 1 tx (no deploy for non-controller).
        let status: TxPoolStatus = client
            .request("txpool_status", Vec::<serde_json::Value>::new())
            .await
            .unwrap();
        assert_eq!(status.pending, 1, "pool should still have only 1 tx");
    }

    // -----------------------------------------------------------------------
    // Case 4: estimate_fee with undeployed Controller
    // -----------------------------------------------------------------------
    {
        use starknet::core::types::{
            BlockId, BlockTag, BroadcastedInvokeTransactionV3, BroadcastedTransaction,
            ResourceBounds, ResourceBoundsMapping, SimulationFlagForEstimateFee,
        };
        use starknet::providers::Provider;

        let provider = node.starknet_provider();

        let invoke_tx = BroadcastedTransaction::Invoke(BroadcastedInvokeTransactionV3 {
            sender_address: Felt::from_hex_unchecked(non_controller_address),
            calldata: vec![Felt::ONE],
            signature: vec![],
            nonce: Felt::ZERO,
            resource_bounds: ResourceBoundsMapping {
                l1_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
                l1_data_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
                l2_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
            },
            tip: 0,
            paymaster_data: vec![],
            account_deployment_data: vec![],
            nonce_data_availability_mode: starknet::core::types::DataAvailabilityMode::L1,
            fee_data_availability_mode: starknet::core::types::DataAvailabilityMode::L1,
            is_query: true,
        });

        let result = provider
            .estimate_fee(
                vec![invoke_tx],
                vec![SimulationFlagForEstimateFee::SkipValidate],
                BlockId::Tag(BlockTag::PreConfirmed),
            )
            .await;

        // For a non-controller, the middleware doesn't prepend a deploy tx.
        // The estimate either succeeds with 1 result or fails with an execution error
        // (since 0xbeef isn't a real contract), but it should NOT fail with a
        // "Controller deployment" error from the middleware.
        match result {
            Ok(estimates) => {
                assert_eq!(estimates.len(), 1, "should return 1 estimate");
            }
            Err(e) => {
                let err_str = e.to_string();
                assert!(
                    !err_str.contains("Controller deployment"),
                    "middleware should not produce deployment error for non-controller: {err_str}"
                );
            }
        }
    }
}
