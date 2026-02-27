#![cfg(feature = "cartridge")]

//! Integration tests for the `ControllerDeploymentService` middleware.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::post;
use axum::Router;
use jsonrpsee::core::middleware::{Batch, Notification, RpcServiceT};
use jsonrpsee::types::Request;
use jsonrpsee::MethodResponse;
use katana_chain_spec::ChainSpec;
use katana_executor::ExecutionFlags;
use katana_gas_price_oracle::GasPriceOracle;
use katana_pool::api::TransactionPool;
use katana_pool::ordering::FiFo;
use katana_pool::pool::Pool;
use katana_pool::validation::NoopValidator;
use katana_primitives::transaction::ExecutableTxWithHash;
use katana_primitives::Felt;
use katana_provider::test_utils::test_provider;
use katana_rpc_server::middleware::cartridge::ControllerDeploymentLayer;
use katana_rpc_server::starknet::{PendingBlockProvider, StarknetApi, StarknetApiConfig};
use katana_rpc_types::*;
use katana_tasks::TaskManager;
use serde_json::json;
use starknet::signers::SigningKey;
use tokio::net::TcpListener;
use tower::Layer;
use url::Url;

// ---------------------------------------------------------------------------
// Group 1: starknet_estimateFee
// ---------------------------------------------------------------------------

/// ## Case:
///
/// The sender address 0x1 already exists and requires no extra deployment.
///
/// ## Expected:
///
/// Since no Controllers need deployment, the request is forwarded unchanged
/// and the response is passed through.
#[tokio::test(flavor = "multi_thread")]
async fn estimate_fee_forwards_when_no_controllers() {
    let inner_responses = {
        let mut m = HashMap::new();
        m.insert(
            "starknet_estimateFee".to_string(),
            vec![FeeEstimate {
                l1_gas_consumed: 1,
                l1_gas_price: 2,
                l2_gas_consumed: 3,
                l2_gas_price: 4,
                l1_data_gas_consumed: 5,
                l1_data_gas_price: 6,
                overall_fee: 7,
            }],
        );
        m
    };

    let setup = setup_test(HashMap::new(), inner_responses).await;

    let tx = make_invoke_tx_json(DEPLOYER_ADDRESS);
    let params = json!([[tx], [], "latest"]);
    let raw = make_rpc_request_str("starknet_estimateFee", &params);

    let request: Request<'_> = serde_json::from_str(&raw).unwrap();
    let response = setup.service.call(request).await;

    // The inner service should have been called exactly once.
    let calls = setup.mock_rpc.recorded_calls();
    assert_eq!(calls.len(), 1, "inner service should be called once");
    assert_eq!(calls[0].method, "starknet_estimateFee");

    // The response should contain the fee estimate from the inner service (passed through).
    let response_json: serde_json::Value = serde_json::from_str(response.as_json().get()).unwrap();
    let result = response_json.get("result").expect("response should have result");
    assert!(result.is_array());
    assert_eq!(result.as_array().unwrap().len(), 1);
}

/// ## Case:
///
/// Address 0xDEAD is not yet deployed and belongs to a Controller account.
///
/// ## Expected:
///
/// The middleware prepends a deploy transaction to the estimate fee
/// request and returns estimates for the original transactions only.
#[tokio::test(flavor = "multi_thread")]
async fn estimate_fee_prepends_deploy_tx_for_controller() {
    let cartridge_responses = {
        let mut m = HashMap::new();
        m.insert(CONTROLLER_ADDRESS.to_string(), controller_calldata_response(CONTROLLER_ADDRESS));
        m
    };

    let inner_responses = {
        let mut m = HashMap::new();
        // The inner service will receive 2 txs (1 deploy + 1 original).
        m.insert(
            "starknet_estimateFee".to_string(),
            vec![
                FeeEstimate {
                    l1_gas_consumed: 0xa,
                    l1_gas_price: 0xb,
                    l2_gas_consumed: 0xc,
                    l2_gas_price: 0xd,
                    l1_data_gas_consumed: 0xe,
                    l1_data_gas_price: 0xf,
                    overall_fee: 0x10,
                },
                FeeEstimate {
                    l1_gas_consumed: 1,
                    l1_gas_price: 2,
                    l2_gas_consumed: 3,
                    l2_gas_price: 4,
                    l1_data_gas_consumed: 5,
                    l1_data_gas_price: 6,
                    overall_fee: 7,
                },
            ],
        );
        m
    };

    let setup = setup_test(cartridge_responses, inner_responses).await;

    let tx = make_invoke_tx_json(CONTROLLER_ADDRESS);
    let params = json!([[tx], [], "latest"]);
    let raw = make_rpc_request_str("starknet_estimateFee", &params);

    let request: Request<'_> = serde_json::from_str(&raw).unwrap();
    let response = setup.service.call(request).await;

    // Inner service should receive 2 txs: deploy tx + original tx.
    let calls = setup.mock_rpc.recorded_calls();
    assert_eq!(calls.len(), 1, "inner service should be called once");
    assert_eq!(
        calls[0].tx_count,
        Some(2),
        "inner service should receive 2 transactions (deploy + original)"
    );

    // The middleware response should have 1 zero-fee estimate (for the original tx only).
    let response_json: serde_json::Value = serde_json::from_str(response.as_json().get()).unwrap();
    let result = response_json.get("result").expect("response should have result");
    let estimates = result.as_array().unwrap();
    assert_eq!(estimates.len(), 1, "response should have 1 estimate for the original tx");

    // All fee fields should be zero.
    let est = &estimates[0];
    assert_eq!(est["overall_fee"], "0x0");
    assert_eq!(est["l1_gas_consumed"], "0x0");
    assert_eq!(est["l2_gas_consumed"], "0x0");
}

/// ## Case:
///
/// Address 0xBEEF is not deployed and the Cartridge API does not recognize it as a
/// Controller.
///
/// ## Expected:
///
/// Even though the address is undeployed, no deploy transaction is created and the original request
/// is forwarded unchanged.
#[tokio::test(flavor = "multi_thread")]
async fn estimate_fee_forwards_for_non_controller() {
    let inner_responses = {
        let mut m = HashMap::new();
        m.insert(
            "starknet_estimateFee".to_string(),
            vec![FeeEstimate {
                l1_gas_consumed: 1,
                l1_gas_price: 2,
                l2_gas_consumed: 3,
                l2_gas_price: 4,
                l1_data_gas_consumed: 5,
                l1_data_gas_price: 6,
                overall_fee: 7,
            }],
        );
        m
    };

    let setup = setup_test(HashMap::new(), inner_responses).await;

    let tx = make_invoke_tx_json(NON_CONTROLLER_ADDRESS);
    let params = json!([[tx], [], "latest"]);
    let raw = make_rpc_request_str("starknet_estimateFee", &params);

    let request: Request<'_> = serde_json::from_str(&raw).unwrap();
    let response = setup.service.call(request).await;

    // Inner service receives the request unchanged (1 tx).
    let calls = setup.mock_rpc.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].method, "starknet_estimateFee");

    // Response is passed through.
    let response_json: serde_json::Value = serde_json::from_str(response.as_json().get()).unwrap();
    let result = response_json.get("result").expect("response should have result");
    assert_eq!(result.as_array().unwrap().len(), 1);
}

/// ## Case:
///
/// Three invoke transactions all from undeployed Controller address 0xDEAD.
///
/// ## Expected:
///
/// The middleware deduplicates by sender address, creating only one deploy transaction
/// despite three transactions from the same sender.
///
/// Inner service receives 4 txs (1 deploy + 3 original); middleware returns 3 zero-fee estimates.
#[tokio::test(flavor = "multi_thread")]
async fn estimate_fee_deduplicates_same_controller() {
    let cartridge_responses = {
        let mut m = HashMap::new();
        m.insert(CONTROLLER_ADDRESS.to_string(), controller_calldata_response(CONTROLLER_ADDRESS));
        m
    };

    let zero_fee = FeeEstimate {
        l1_gas_consumed: 0,
        l1_gas_price: 0,
        l2_gas_consumed: 0,
        l2_gas_price: 0,
        l1_data_gas_consumed: 0,
        l1_data_gas_price: 0,
        overall_fee: 0,
    };

    let inner_responses = {
        let mut m = HashMap::new();
        // Inner service receives 4 txs (1 deploy + 3 original).
        m.insert(
            "starknet_estimateFee".to_string(),
            vec![zero_fee.clone(), zero_fee.clone(), zero_fee.clone(), zero_fee],
        );
        m
    };

    let setup = setup_test(cartridge_responses, inner_responses).await;

    let tx1 = make_invoke_tx_json(CONTROLLER_ADDRESS);
    let tx2 = make_invoke_tx_json(CONTROLLER_ADDRESS);
    let tx3 = make_invoke_tx_json(CONTROLLER_ADDRESS);
    let params = json!([[tx1, tx2, tx3], [], "latest"]);
    let raw = make_rpc_request_str("starknet_estimateFee", &params);

    let request: Request<'_> = serde_json::from_str(&raw).unwrap();
    let response = setup.service.call(request).await;

    // Inner service should receive 4 txs: 1 deploy + 3 original.
    let calls = setup.mock_rpc.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].tx_count,
        Some(4),
        "inner service should receive 4 transactions (1 deploy + 3 original)"
    );

    // Middleware should return 3 zero-fee estimates (one per original tx).
    let response_json: serde_json::Value = serde_json::from_str(response.as_json().get()).unwrap();
    let result = response_json.get("result").expect("response should have result");
    let estimates = result.as_array().unwrap();
    assert_eq!(estimates.len(), 3, "response should have 3 estimates for the 3 original txs");

    for est in estimates {
        assert_eq!(est["overall_fee"], "0x0");
    }
}

// ---------------------------------------------------------------------------
// Group 2: cartridge_addExecuteFromOutside
// ---------------------------------------------------------------------------

/// ## Case:
///
/// The sender address (0x1) is already deployed.
///
/// ## Expected:
///
/// The middleware detects this and skips Controller deployment, forwarding the
/// request unchanged without querying the Cartridge API.
///
/// Inner service receives request unchanged; pool remains empty; Cartridge API receives no
/// requests.
#[tokio::test(flavor = "multi_thread")]
async fn execute_outside_skips_deploy_when_already_deployed() {
    let setup = setup_test(HashMap::new(), HashMap::new()).await;

    let params = make_execute_outside_params(DEPLOYER_ADDRESS);
    let raw = make_rpc_request_str("cartridge_addExecuteFromOutside", &params);

    let request: Request<'_> = serde_json::from_str(&raw).unwrap();
    let _response = setup.service.call(request).await;

    // Pool should be empty — no deploy tx was added.
    assert_eq!(setup.pool.size(), 0, "pool should be empty");

    // Inner service should have been called (request forwarded).
    let calls = setup.mock_rpc.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].method, "cartridge_addExecuteFromOutside");

    // Cartridge API should not have been queried.
    let api_requests = setup.mock_api_state.received_requests.lock().unwrap();
    assert!(api_requests.is_empty(), "Cartridge API should not have been queried");
}

/// ## Case:
///
/// The sender address (0xDEAD) is not deployed and belongs to a Controller account.
///
/// ## Expected:
///
/// The middleware creates a deploy transaction, adds it to the pool, and then forwards
/// the original request to the inner service.
///
/// Pool contains 1 deploy transaction; inner service receives request.
#[tokio::test(flavor = "multi_thread")]
async fn execute_outside_deploys_controller() {
    let cartridge_responses = {
        let mut m = HashMap::new();
        m.insert(CONTROLLER_ADDRESS.to_string(), controller_calldata_response(CONTROLLER_ADDRESS));
        m
    };

    let setup = setup_test(cartridge_responses, HashMap::new()).await;

    let params = make_execute_outside_params(CONTROLLER_ADDRESS);
    let raw = make_rpc_request_str("cartridge_addExecuteFromOutside", &params);

    let request: Request<'_> = serde_json::from_str(&raw).unwrap();
    let _response = setup.service.call(request).await;

    // A deploy transaction should have been added to the pool.
    assert_eq!(setup.pool.size(), 1, "pool should contain 1 deploy transaction");

    // Inner service should have been called (request forwarded).
    let calls = setup.mock_rpc.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].method, "cartridge_addExecuteFromOutside");
}

/// ## Case:
///
/// The sender address (0xBEEF) is not deployed and is not a Controller.
///
/// ## Expected:
///
/// The middleware skips deployment and forwards the request unchanged.
///
/// Pool remains empty; inner service receives request.
#[tokio::test(flavor = "multi_thread")]
async fn execute_outside_skips_deploy_for_non_controller() {
    let setup = setup_test(HashMap::new(), HashMap::new()).await;

    let params = make_execute_outside_params(NON_CONTROLLER_ADDRESS);
    let raw = make_rpc_request_str("cartridge_addExecuteFromOutside", &params);

    let request: Request<'_> = serde_json::from_str(&raw).unwrap();
    let _response = setup.service.call(request).await;

    // Pool should be empty — no deploy tx was added.
    assert_eq!(setup.pool.size(), 0, "pool should be empty");

    // Inner service should have been called.
    let calls = setup.mock_rpc.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].method, "cartridge_addExecuteFromOutside");
}

/// ## Case:
///
/// Same scenario as `execute_outside_deploys_controller` but uses the alternate
/// method name "cartridge_addExecuteOutsideTransaction" to verify both method
/// names are intercepted by the middleware.
///
/// ## Expected:
///
/// Deploy transaction added to pool and request forwarded.
#[tokio::test(flavor = "multi_thread")]
async fn execute_outside_tx_method_variant() {
    let cartridge_responses = {
        let mut m = HashMap::new();
        m.insert(CONTROLLER_ADDRESS.to_string(), controller_calldata_response(CONTROLLER_ADDRESS));
        m
    };

    let setup = setup_test(cartridge_responses, HashMap::new()).await;

    let params = make_execute_outside_params(CONTROLLER_ADDRESS);
    let raw = make_rpc_request_str("cartridge_addExecuteOutsideTransaction", &params);

    let request: Request<'_> = serde_json::from_str(&raw).unwrap();
    let _response = setup.service.call(request).await;

    // A deploy transaction should have been added to the pool.
    assert_eq!(setup.pool.size(), 1, "pool should contain 1 deploy transaction");

    // Inner service should have been called with the alternate method name.
    let calls = setup.mock_rpc.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].method, "cartridge_addExecuteOutsideTransaction");
}

// ---------------------------------------------------------------------------
// Group 3: Passthrough
// ---------------------------------------------------------------------------

/// ## Case:
///
/// A request for "starknet_getBlockNumber" is not intercepted by the middleware
/// and is forwarded directly to the inner service.
///
/// ## Expected:
///
/// inner service receives request unchanged; no Cartridge API requests made.
#[tokio::test(flavor = "multi_thread")]
async fn passthrough_other_methods() {
    let setup = setup_test(HashMap::new(), HashMap::new()).await;

    let raw = make_rpc_request_str("starknet_getBlockNumber", &json!([]));

    let request: Request<'_> = serde_json::from_str(&raw).unwrap();
    let _response = setup.service.call(request).await;

    let calls = setup.mock_rpc.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].method, "starknet_getBlockNumber");

    let api_requests = setup.mock_api_state.received_requests.lock().unwrap();
    assert!(api_requests.is_empty(), "Cartridge API should not have been queried");
}

/// ## Case:
///
/// When starknet_estimateFee is called with malformed params, the middleware
/// should gracefully falls through to the inner service rather than erroring.
///
/// ## Expected:
///
/// Inner service receives request unchanged.
#[tokio::test(flavor = "multi_thread")]
async fn passthrough_malformed_estimate_fee() {
    let setup = setup_test(HashMap::new(), HashMap::new()).await;

    // Malformed params — not a valid array of transactions.
    let raw = make_rpc_request_str("starknet_estimateFee", &json!(["not_valid"]));

    let request: Request<'_> = serde_json::from_str(&raw).unwrap();
    let _response = setup.service.call(request).await;

    // The inner service should have received the request (fallthrough).
    let calls = setup.mock_rpc.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].method, "starknet_estimateFee");
}

// ---------------------------------------------------------------------------
// Test Fixtures
// ---------------------------------------------------------------------------

type TestPool =
    Pool<ExecutableTxWithHash, NoopValidator<ExecutableTxWithHash>, FiFo<ExecutableTxWithHash>>;

/// A no-op pending block provider. All methods return `Ok(None)`, matching
/// instant-mining mode behaviour.
#[derive(Debug, Clone)]
struct NoPendingBlockProvider;

impl PendingBlockProvider for NoPendingBlockProvider {
    fn pending_state(
        &self,
    ) -> katana_rpc_server::starknet::StarknetApiResult<
        Option<Box<dyn katana_provider::api::state::StateProvider>>,
    > {
        Ok(None)
    }

    fn get_pending_state_update(
        &self,
    ) -> katana_rpc_server::starknet::StarknetApiResult<Option<PreConfirmedStateUpdate>> {
        Ok(None)
    }

    fn get_pending_block_with_txs(
        &self,
    ) -> katana_rpc_server::starknet::StarknetApiResult<Option<PreConfirmedBlockWithTxs>> {
        Ok(None)
    }

    fn get_pending_block_with_receipts(
        &self,
    ) -> katana_rpc_server::starknet::StarknetApiResult<Option<PreConfirmedBlockWithReceipts>> {
        Ok(None)
    }

    fn get_pending_block_with_tx_hashes(
        &self,
    ) -> katana_rpc_server::starknet::StarknetApiResult<Option<PreConfirmedBlockWithTxHashes>> {
        Ok(None)
    }

    fn get_pending_transaction(
        &self,
        _hash: katana_primitives::transaction::TxHash,
    ) -> katana_rpc_server::starknet::StarknetApiResult<Option<RpcTxWithHash>> {
        Ok(None)
    }

    fn get_pending_receipt(
        &self,
        _hash: katana_primitives::transaction::TxHash,
    ) -> katana_rpc_server::starknet::StarknetApiResult<Option<TxReceiptWithBlockInfo>> {
        Ok(None)
    }

    fn get_pending_trace(
        &self,
        _hash: katana_primitives::transaction::TxHash,
    ) -> katana_rpc_server::starknet::StarknetApiResult<Option<TxTrace>> {
        Ok(None)
    }

    fn get_pending_transaction_by_index(
        &self,
        _index: katana_primitives::transaction::TxNumber,
    ) -> katana_rpc_server::starknet::StarknetApiResult<Option<RpcTxWithHash>> {
        Ok(None)
    }
}

#[derive(Clone)]
struct MockCartridgeApiState {
    /// Map from hex address (with "0x" prefix, lowercase) to the response JSON.
    responses: Arc<HashMap<String, serde_json::Value>>,
    /// Log of all requests received.
    received_requests: Arc<Mutex<Vec<serde_json::Value>>>,
}

async fn mock_cartridge_handler(
    State(state): State<MockCartridgeApiState>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    state.received_requests.lock().unwrap().push(body.clone());

    let address = body.get("address").and_then(|v| v.as_str()).unwrap_or("");

    if let Some(response) = state.responses.get(address) {
        axum::Json(response.clone()).into_response()
    } else {
        "Address not found".into_response()
    }
}

/// Start a mock Cartridge API server. Returns (base URL, state handle, join handle).
async fn start_mock_cartridge_api(
    responses: HashMap<String, serde_json::Value>,
) -> (Url, MockCartridgeApiState) {
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

    let url = Url::parse(&format!("http://{addr}")).unwrap();
    (url, state)
}

// ---------------------------------------------------------------------------
// Mock inner RPC service
// ---------------------------------------------------------------------------

/// A recorded call to the mock RPC service.
#[derive(Clone, Debug)]
struct RecordedCall {
    method: String,
    /// For estimate_fee, how many transactions were in the params.
    tx_count: Option<usize>,
}

#[derive(Clone)]
struct MockRpcService {
    /// Records all calls.
    calls: Arc<Mutex<Vec<RecordedCall>>>,
    /// Pre-configured response JSON per method name.
    responses: Arc<HashMap<String, Vec<FeeEstimate>>>,
}

impl MockRpcService {
    fn new(responses: HashMap<String, Vec<FeeEstimate>>) -> Self {
        Self { calls: Arc::new(Mutex::new(Vec::new())), responses: Arc::new(responses) }
    }

    fn recorded_calls(&self) -> Vec<RecordedCall> {
        self.calls.lock().unwrap().clone()
    }
}

impl RpcServiceT for MockRpcService {
    type MethodResponse = MethodResponse;
    type BatchResponse = MethodResponse;
    type NotificationResponse = MethodResponse;

    fn call<'a>(
        &self,
        request: Request<'a>,
    ) -> impl Future<Output = Self::MethodResponse> + Send + 'a {
        let method = request.method_name().to_owned();

        // Try to count transactions if this is an estimate_fee request.
        let params = request.params();
        let tx_count = if method == "starknet_estimateFee" {
            // Parse the first param (array of txs) from the sequence params.
            let mut seq = params.sequence();
            let txs: Result<Vec<serde_json::Value>, _> = seq.next();
            txs.ok().map(|v| v.len())
        } else {
            None
        };

        self.calls.lock().unwrap().push(RecordedCall { method: method.clone(), tx_count });

        let response = if let Some(resp) = self.responses.get(&method) {
            MethodResponse::response(
                request.id().clone(),
                jsonrpsee::ResponsePayload::success(resp.clone()),
                usize::MAX,
            )
        } else {
            MethodResponse::response(
                request.id().clone(),
                jsonrpsee::ResponsePayload::success(serde_json::Value::Null),
                usize::MAX,
            )
        };

        std::future::ready(response)
    }

    fn batch<'a>(
        &self,
        _requests: Batch<'a>,
    ) -> impl Future<Output = Self::BatchResponse> + Send + 'a {
        std::future::ready(MethodResponse::response(
            jsonrpsee::types::Id::Null,
            jsonrpsee::ResponsePayload::success(serde_json::Value::Null),
            usize::MAX,
        ))
    }

    fn notification<'a>(
        &self,
        _n: Notification<'a>,
    ) -> impl Future<Output = Self::NotificationResponse> + Send + 'a {
        std::future::ready(MethodResponse::response(
            jsonrpsee::types::Id::Null,
            jsonrpsee::ResponsePayload::success(serde_json::Value::Null),
            usize::MAX,
        ))
    }
}

/// An undeployed address that the mock API will recognize as a Controller.
const CONTROLLER_ADDRESS: &str = "0xdead";
/// An undeployed address that the mock API will NOT recognize as a Controller.
const NON_CONTROLLER_ADDRESS: &str = "0xbeef";
/// The deployer address — matches the genesis account at 0x1 in test_provider.
const DEPLOYER_ADDRESS: &str = "0x1";

/// Builds a `serde_json::Value` response for the Cartridge API that represents
/// a valid Controller account with some dummy constructor calldata.
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

/// Creates a valid V3 invoke transaction JSON for the given sender address.
fn make_invoke_tx_json(sender_address: &str) -> serde_json::Value {
    json!({
        "type": "INVOKE",
        "version": "0x3",
        "sender_address": sender_address,
        "calldata": ["0x1"],
        "signature": ["0x0"],
        "nonce": "0x0",
        "resource_bounds": {
            "l1_gas": { "max_amount": "0x0", "max_price_per_unit": "0x0" },
            "l2_gas": { "max_amount": "0x0", "max_price_per_unit": "0x0" },
            "l1_data_gas": { "max_amount": "0x0", "max_price_per_unit": "0x0" }
        },
        "tip": "0x0",
        "paymaster_data": [],
        "account_deployment_data": [],
        "nonce_data_availability_mode": "L1",
        "fee_data_availability_mode": "L1"
    })
}

/// Creates a JSON-RPC 2.0 request string and constructs the corresponding `Request<'_>`.
fn make_rpc_request_str(method: &str, params: &serde_json::Value) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params
    })
    .to_string()
}

/// A complete test setup context.
struct TestSetup {
    service: <ControllerDeploymentLayer<
        TestPool,
        NoPendingBlockProvider,
        katana_provider::DbProviderFactory,
    > as Layer<MockRpcService>>::Service,
    mock_rpc: MockRpcService,
    mock_api_state: MockCartridgeApiState,
    pool: TestPool,
}

async fn setup_test(
    cartridge_api_responses: HashMap<String, serde_json::Value>,
    inner_rpc_responses: HashMap<String, Vec<FeeEstimate>>,
) -> TestSetup {
    let (mock_url, mock_api_state) = start_mock_cartridge_api(cartridge_api_responses).await;

    let chain_spec = Arc::new(ChainSpec::dev());
    let pool = Pool::new(NoopValidator::new(), FiFo::new());
    let task_spawner = TaskManager::current().task_spawner();
    let gas_oracle = GasPriceOracle::create_for_testing();
    let storage = test_provider();

    let config = StarknetApiConfig {
        max_event_page_size: None,
        max_proof_keys: None,
        max_call_gas: None,
        max_concurrent_estimate_fee_requests: None,
        simulation_flags: ExecutionFlags::new().with_fee(false).with_account_validation(false),
        versioned_constant_overrides: None,
    };

    let starknet_api = StarknetApi::new(
        chain_spec,
        pool.clone(),
        task_spawner,
        NoPendingBlockProvider,
        gas_oracle,
        config,
        storage,
    );

    let cartridge_api = ::cartridge::CartridgeApiClient::new(mock_url);

    let deployer_address = Felt::from(1u64).into();
    let deployer_private_key = SigningKey::from_secret_scalar(Felt::from(1u64));

    let layer = ControllerDeploymentLayer::new(
        starknet_api,
        cartridge_api,
        deployer_address,
        deployer_private_key,
    );

    let mock_rpc = MockRpcService::new(inner_rpc_responses);
    let service = layer.layer(mock_rpc.clone());

    TestSetup { service, mock_rpc, mock_api_state, pool }
}

fn make_execute_outside_params(address: &str) -> serde_json::Value {
    json!([
        address,
        {
            "caller": "0x414e595f43414c4c4552",
            "nonce": "0x1",
            "execute_after": "0x0",
            "execute_before": "0xffffffffffffffff",
            "calls": [{
                "to": "0x1",
                "selector": "0x2",
                "calldata": ["0x3"]
            }]
        },
        ["0x0", "0x0"],
        null
    ])
}
