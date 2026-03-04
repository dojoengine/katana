#![cfg(feature = "cartridge")]

//! Integration tests for the `ControllerDeploymentService` middleware.

use std::collections::HashMap;

use jsonrpsee::MethodResponse;
use katana_pool::api::TransactionPool;
use katana_rpc_types::FeeEstimate;
use katana_utils::arbitrary;
use serde::de::DeserializeOwned;
use serde_json::json;
use setup::*;

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
    let expected_estimates = vec![arbitrary!(FeeEstimate)];

    let inner_responses =
        HashMap::from_iter([("starknet_estimateFee".to_string(), expected_estimates.clone())]);

    let setup = setup_test(HashMap::new(), inner_responses).await;

    let tx = make_invoke_tx(DEPLOYER_ADDRESS);
    let params = json!([[tx], [], "latest"]);
    let response = setup.call("starknet_estimateFee", &params).await;

    // The inner service should have been called exactly once.
    let calls = setup.mock_rpc.recorded_calls();
    assert_eq!(calls.len(), 1, "inner service should be called once");
    assert_eq!(calls[0].method, "starknet_estimateFee");

    // The response should contain the fee estimate from the inner service (passed through).
    let actual_estimates: Vec<FeeEstimate> = get_result(response);

    assert_eq!(actual_estimates.len(), 1, "should only have 1 estimate");
    assert_eq!(actual_estimates, expected_estimates);
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
    let expected_estimates = vec![arbitrary!(FeeEstimate), arbitrary!(FeeEstimate)];

    let cartridge_responses = HashMap::from_iter([(
        CONTROLLER_ADDRESS.to_string(),
        controller_calldata_response(CONTROLLER_ADDRESS),
    )]);

    // The inner service will receive 2 txs (1 deploy + 1 original).
    let inner_responses =
        HashMap::from_iter([("starknet_estimateFee".to_string(), expected_estimates.clone())]);

    let setup = setup_test(cartridge_responses, inner_responses).await;

    let tx = make_invoke_tx(CONTROLLER_ADDRESS);
    let params = json!([[tx], [], "latest"]);
    let response = setup.call("starknet_estimateFee", &params).await;

    // Inner service should receive 2 txs: deploy tx + original tx.
    let calls = setup.mock_rpc.recorded_calls();
    assert_eq!(calls.len(), 1, "inner service should be called once");
    assert_eq!(
        calls[0].tx_count,
        Some(2),
        "inner service should receive 2 transactions (deploy + original)"
    );

    // The middleware response should have 1 zero-fee estimate (for the original tx only).
    let actual_estimates: Vec<FeeEstimate> = get_result(response);

    assert_eq!(actual_estimates.len(), 1, "response should have 1 estimate for the original tx");
    assert_eq!(actual_estimates[..], expected_estimates[1..]);
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
    let expected_estimates = vec![arbitrary!(FeeEstimate)];

    let inner_responses =
        HashMap::from_iter([("starknet_estimateFee".to_string(), expected_estimates.clone())]);

    let setup = setup_test(HashMap::new(), inner_responses).await;

    let tx = make_invoke_tx(NON_CONTROLLER_ADDRESS);
    let params = json!([[tx], [], "latest"]);
    let response = setup.call("starknet_estimateFee", &params).await;

    // Inner service receives the request unchanged (1 tx).
    let calls = setup.mock_rpc.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].method, "starknet_estimateFee");

    // Response is passed through.
    let actual_estimates: Vec<FeeEstimate> = get_result(response);

    assert_eq!(actual_estimates.len(), 1, "should only have 1 estimate");
    assert_eq!(actual_estimates, expected_estimates);
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
    let expected_estimates = vec![
        arbitrary!(FeeEstimate),
        arbitrary!(FeeEstimate),
        arbitrary!(FeeEstimate),
        arbitrary!(FeeEstimate),
    ];

    let cartridge_responses = HashMap::from_iter([(
        CONTROLLER_ADDRESS.to_string(),
        controller_calldata_response(CONTROLLER_ADDRESS),
    )]);

    // Inner service receives 4 txs (1 deploy + 3 original).
    let inner_responses =
        HashMap::from_iter([("starknet_estimateFee".to_string(), expected_estimates.clone())]);

    let setup = setup_test(cartridge_responses, inner_responses).await;

    let tx1 = make_invoke_tx(CONTROLLER_ADDRESS);
    let tx2 = make_invoke_tx(CONTROLLER_ADDRESS);
    let tx3 = make_invoke_tx(CONTROLLER_ADDRESS);
    let params = json!([[tx1, tx2, tx3], [], "latest"]);
    let response = setup.call("starknet_estimateFee", &params).await;

    // Inner service should receive 4 txs: 1 deploy + 3 original.
    let calls = setup.mock_rpc.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].tx_count,
        Some(4),
        "inner service should receive 4 transactions (1 deploy + 3 original)"
    );

    // Middleware should return 3 zero-fee estimates (one per original tx).
    let actual_estimates: Vec<FeeEstimate> = get_result(response);

    assert_eq!(
        actual_estimates.len(),
        3,
        "response should have 3 estimates for the 3 original txs"
    );

    assert_eq!(actual_estimates[..], expected_estimates[1..]);
}

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
    setup.call("cartridge_addExecuteFromOutside", &params).await;

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
    let cartridge_responses = HashMap::from_iter([(
        CONTROLLER_ADDRESS.to_string(),
        controller_calldata_response(CONTROLLER_ADDRESS),
    )]);

    let setup = setup_test(cartridge_responses, HashMap::new()).await;

    let params = make_execute_outside_params(CONTROLLER_ADDRESS);
    setup.call("cartridge_addExecuteFromOutside", &params).await;

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
    setup.call("cartridge_addExecuteFromOutside", &params).await;

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
    setup.call("cartridge_addExecuteOutsideTransaction", &params).await;

    // A deploy transaction should have been added to the pool.
    assert_eq!(setup.pool.size(), 1, "pool should contain 1 deploy transaction");

    // Inner service should have been called with the alternate method name.
    let calls = setup.mock_rpc.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].method, "cartridge_addExecuteOutsideTransaction");
}

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

    setup.call("starknet_getBlockNumber", &json!([])).await;

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
    setup.call("starknet_estimateFee", &json!(["not_valid"])).await;

    // The inner service should have received the request (fallthrough).
    let calls = setup.mock_rpc.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].method, "starknet_estimateFee");
}

mod setup {
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
    use katana_pool::ordering::FiFo;
    use katana_pool::pool::Pool;
    use katana_pool::validation::NoopValidator;
    use katana_primitives::da::DataAvailabilityMode;
    use katana_primitives::fee::{AllResourceBoundsMapping, ResourceBoundsMapping, Tip};
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

    pub(super) type TestPool =
        Pool<ExecutableTxWithHash, NoopValidator<ExecutableTxWithHash>, FiFo<ExecutableTxWithHash>>;

    /// An undeployed address that the mock API will recognize as a Controller.
    pub(super) const CONTROLLER_ADDRESS: &str = "0xdead";
    /// An undeployed address that the mock API will NOT recognize as a Controller.
    pub(super) const NON_CONTROLLER_ADDRESS: &str = "0xbeef";
    /// The deployer address — matches the genesis account at 0x1 in test_provider.
    pub(super) const DEPLOYER_ADDRESS: &str = "0x1";

    /// A complete test setup context.
    pub(super) struct TestSetup {
        service: <ControllerDeploymentLayer<
            TestPool,
            NoPendingBlockProvider,
            katana_provider::DbProviderFactory,
        > as Layer<MockRpcService>>::Service,
        pub(super) mock_rpc: MockRpcService,
        pub(super) mock_api_state: MockCartridgeApiState,
        pub(super) pool: TestPool,
    }

    impl TestSetup {
        pub(super) async fn call(
            &self,
            method: &str,
            params: &serde_json::Value,
        ) -> MethodResponse {
            let raw = make_rpc_request_str(method, params);
            let request: Request<'_> = serde_json::from_str(&raw).unwrap();
            self.service.call(request).await
        }
    }

    pub(super) async fn setup_test(
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

    /// Builds a `serde_json::Value` response for the Cartridge API that represents
    /// a valid Controller account with some dummy constructor calldata.
    pub(super) fn controller_calldata_response(address: &str) -> serde_json::Value {
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

    /// Creates a valid V3 invoke transaction for the given sender address.
    pub(super) fn make_invoke_tx(sender_address: &str) -> BroadcastedTx {
        BroadcastedTx::Invoke(BroadcastedInvokeTx {
            sender_address: Felt::from_hex_unchecked(sender_address).into(),
            calldata: vec![Felt::ONE],
            signature: vec![Felt::ZERO],
            nonce: Felt::ZERO,
            resource_bounds: ResourceBoundsMapping::All(AllResourceBoundsMapping {
                l1_gas: Default::default(),
                l2_gas: Default::default(),
                l1_data_gas: Default::default(),
            }),
            tip: Tip::default(),
            paymaster_data: vec![],
            account_deployment_data: vec![],
            nonce_data_availability_mode: DataAvailabilityMode::L1,
            fee_data_availability_mode: DataAvailabilityMode::L1,
            is_query: false,
        })
    }

    pub(super) fn make_execute_outside_params(address: &str) -> serde_json::Value {
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

    // ---- internal helpers ----

    fn make_rpc_request_str(method: &str, params: &serde_json::Value) -> String {
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params
        })
        .to_string()
    }

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

    // ---- mock types ----

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
        ) -> katana_rpc_server::starknet::StarknetApiResult<Option<PreConfirmedStateUpdate>>
        {
            Ok(None)
        }

        fn get_pending_block_with_txs(
            &self,
        ) -> katana_rpc_server::starknet::StarknetApiResult<Option<PreConfirmedBlockWithTxs>>
        {
            Ok(None)
        }

        fn get_pending_block_with_receipts(
            &self,
        ) -> katana_rpc_server::starknet::StarknetApiResult<Option<PreConfirmedBlockWithReceipts>>
        {
            Ok(None)
        }

        fn get_pending_block_with_tx_hashes(
            &self,
        ) -> katana_rpc_server::starknet::StarknetApiResult<Option<PreConfirmedBlockWithTxHashes>>
        {
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
        ) -> katana_rpc_server::starknet::StarknetApiResult<Option<TxReceiptWithBlockInfo>>
        {
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
    pub(super) struct MockCartridgeApiState {
        /// Map from hex address (with "0x" prefix, lowercase) to the response JSON.
        responses: Arc<HashMap<String, serde_json::Value>>,
        /// Log of all requests received.
        pub(super) received_requests: Arc<Mutex<Vec<serde_json::Value>>>,
    }

    /// A recorded call to the mock RPC service.
    #[derive(Clone, Debug)]
    pub(super) struct RecordedCall {
        pub(super) method: String,
        /// For estimate_fee, how many transactions were in the params.
        pub(super) tx_count: Option<usize>,
    }

    #[derive(Clone)]
    pub(super) struct MockRpcService {
        /// Records all calls.
        calls: Arc<Mutex<Vec<RecordedCall>>>,
        /// Pre-configured response JSON per method name.
        responses: Arc<HashMap<String, Vec<FeeEstimate>>>,
    }

    impl MockRpcService {
        pub(super) fn new(responses: HashMap<String, Vec<FeeEstimate>>) -> Self {
            Self { calls: Arc::new(Mutex::new(Vec::new())), responses: Arc::new(responses) }
        }

        pub(super) fn recorded_calls(&self) -> Vec<RecordedCall> {
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
}

fn get_result<T: DeserializeOwned>(response: MethodResponse) -> T {
    let raw_json = response.into_json();
    let json = serde_json::to_value(raw_json).unwrap();
    serde_json::from_value(json["result"].clone()).unwrap()
}
