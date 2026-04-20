//! Unit tests for the `ControllerDeploymentService` middleware.

use std::collections::HashMap;

use jsonrpsee::MethodResponse;
use katana_pool::api::TransactionPool;
use katana_primitives::felt;
use katana_rpc_types::FeeEstimate;
use katana_utils::arbitrary;
use serde::de::DeserializeOwned;
use serde_json::json;
use setup::*;

/// ## Case:
///
/// The sender address 0x1 is already deployed and requires no extra deployment.
///
/// ## Expected:
///
/// The request is forwarded unchanged and the response is passed through.
#[tokio::test(flavor = "multi_thread")]
async fn estimate_fee_forwards_when_deployed_account() {
    let expected_estimates = vec![arbitrary!(FeeEstimate)];

    let rpc_responses = HashMap::from_iter([("starknet_estimateFee", expected_estimates.clone())]);
    let test = setup_test(HashMap::new(), rpc_responses).await;

    let tx = make_invoke_tx(DEPLOYER_ADDRESS);
    let response = test.call("starknet_estimateFee", &json!([[tx], [], "latest"])).await;

    // If it is deployed, a getAccountCalldata request should not be made.
    assert!(!test.mock_api_state.has_get_account_calldata_request(DEPLOYER_ADDRESS));

    let calls = test.rpc.estimate_fee_calls();
    assert_eq!(calls.len(), 1, "expected only one call to estimateFee");
    assert_eq!(calls[0].tx_count, 1, "rpc service should get 1 tx (no deploy)");

    let calls_count = test.rpc.outside_execute_calls_count();
    assert_eq!(calls_count, 0, "rpc service should not receive any outside execute calls");

    let actual_estimates: Vec<FeeEstimate> = get_result(response);
    assert_eq!(actual_estimates, expected_estimates, "response is passed through as is");
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
    let cartridge_responses = HashMap::from_iter([(
        CONTROLLER_ADDRESS,
        controller_calldata_response(CONTROLLER_ADDRESS),
    )]);

    // The rpc service will receive 2 txs (1 deploy + 1 original).
    let expected_estimates = vec![arbitrary!(FeeEstimate), arbitrary!(FeeEstimate)];
    let rpc_responses = HashMap::from_iter([("starknet_estimateFee", expected_estimates.clone())]);

    let test = setup_test(cartridge_responses, rpc_responses).await;

    let tx = make_invoke_tx(CONTROLLER_ADDRESS);
    let response = test.call("starknet_estimateFee", &json!([[tx], [], "latest"])).await;

    // If it is undeployed, a getAccountCalldata request should be made.
    assert!(test.mock_api_state.has_get_account_calldata_request(CONTROLLER_ADDRESS));

    let calls = test.rpc.estimate_fee_calls();
    assert_eq!(calls.len(), 1, "expected only one call to estimateFee");
    assert_eq!(calls[0].tx_count, 2, "rpc service should receive 2 txs (deploy + original)");

    let calls_count = test.rpc.outside_execute_calls_count();
    assert_eq!(calls_count, 0, "rpc service should not receive any outside execute calls");

    // The middleware should remove the deploy tx estimate and return only the original tx
    // estimate before sending it back to the caller.
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
async fn estimate_fee_forwards_for_undeployed_non_controller() {
    let expected_estimates = vec![arbitrary!(FeeEstimate)];
    let rpc_responses = HashMap::from_iter([("starknet_estimateFee", expected_estimates.clone())]);

    let test = setup_test(HashMap::new(), rpc_responses).await;

    let tx = make_invoke_tx(NON_CONTROLLER_ADDRESS);
    let response = test.call("starknet_estimateFee", &json!([[tx], [], "latest"])).await;

    // If it is undeployed, a getAccountCalldata request should be made regardless if it's a
    // Controller or not.
    assert!(test.mock_api_state.has_get_account_calldata_request(NON_CONTROLLER_ADDRESS));

    let calls = test.rpc.estimate_fee_calls();
    assert_eq!(calls.len(), 1, "expected only one call to estimateFee");
    assert_eq!(calls[0].tx_count, 1, "rpc service should get 1 tx (no deploy)");

    let calls_count = test.rpc.outside_execute_calls_count();
    assert_eq!(calls_count, 0, "rpc service should not receive any outside execute calls");

    let actual_estimates: Vec<FeeEstimate> = get_result(response);
    assert_eq!(actual_estimates, expected_estimates, "response is passed through as is");
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
        arbitrary!(FeeEstimate), // prepended contorller deploy tx
        arbitrary!(FeeEstimate),
        arbitrary!(FeeEstimate),
        arbitrary!(FeeEstimate),
    ];

    let cartridge_responses = HashMap::from_iter([(
        CONTROLLER_ADDRESS,
        controller_calldata_response(CONTROLLER_ADDRESS),
    )]);

    // rpc service receives 4 txs (1 deploy + 3 original).
    let rpc_responses = HashMap::from_iter([("starknet_estimateFee", expected_estimates.clone())]);

    let setup = setup_test(cartridge_responses, rpc_responses).await;

    let tx1 = make_invoke_tx(CONTROLLER_ADDRESS);
    let tx2 = make_invoke_tx(CONTROLLER_ADDRESS);
    let tx3 = make_invoke_tx(CONTROLLER_ADDRESS);
    let res = setup.call("starknet_estimateFee", &json!([[tx1, tx2, tx3], [], "latest"])).await;

    // If it is undeployed, a getAccountCalldata request should be made.
    assert!(setup.mock_api_state.has_get_account_calldata_request(CONTROLLER_ADDRESS));

    let calls = setup.rpc.estimate_fee_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].tx_count, 4, "rpc service should get 4 txs (1 deploy + 3 og)");

    let calls_count = setup.rpc.outside_execute_calls_count();
    assert_eq!(calls_count, 0, "rpc service should not receive any outside execute calls");

    let actual_estimates: Vec<FeeEstimate> = get_result(res);
    assert_eq!(actual_estimates.len(), 3, "response should not include the estimate for deploy tx");
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
/// RPC service receives request unchanged; pool remains empty; Cartridge API receives no
/// requests.
#[tokio::test(flavor = "multi_thread")]
async fn execute_outside_skips_deploy_when_already_deployed() {
    let setup = setup_test(HashMap::new(), HashMap::new()).await;

    let execute_outside = make_execute_outside();
    let params = json!([DEPLOYER_ADDRESS, execute_outside, ["0x0", "0x0"], null]);
    setup.call("cartridge_addExecuteFromOutside", &params).await;

    assert_eq!(setup.pool.size(), 0, "pool should be empty — no deploy tx was added");

    let calls_count = setup.rpc.outside_execute_calls_count();
    assert_eq!(calls_count, 1);
    let calls = setup.rpc.outside_execute_calls_from(DEPLOYER_ADDRESS);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].outside_execution, execute_outside);

    let calls = setup.rpc.estimate_fee_calls();
    assert!(calls.is_empty(), "no calls to estimateFee");

    // Cartridge API should not have been queried because the address is already deployed.
    assert!(!setup.mock_api_state.has_get_account_calldata_request(DEPLOYER_ADDRESS));
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
        CONTROLLER_ADDRESS,
        controller_calldata_response(CONTROLLER_ADDRESS),
    )]);

    let setup = setup_test(cartridge_responses, HashMap::new()).await;

    let execute_outside = make_execute_outside();
    let params = json!([CONTROLLER_ADDRESS, execute_outside, ["0x0", "0x0"], null]);
    setup.call("cartridge_addExecuteFromOutside", &params).await;

    // A deploy transaction should have been added to the pool.
    assert_eq!(setup.pool.size(), 1, "pool should contain 1 deploy transaction");

    // rpc service should have been called (request forwarded).
    let calls_count = setup.rpc.outside_execute_calls_count();
    assert_eq!(calls_count, 1);
    let calls = setup.rpc.outside_execute_calls_from(CONTROLLER_ADDRESS);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].outside_execution, execute_outside);
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

    let execute_outside = make_execute_outside();
    let params = json!([NON_CONTROLLER_ADDRESS, execute_outside, ["0x0", "0x0"], null]);
    setup.call("cartridge_addExecuteFromOutside", &params).await;

    // Pool should be empty — no deploy tx was added.
    assert_eq!(setup.pool.size(), 0, "pool should be empty");

    // rpc service should have been called.
    let calls_count = setup.rpc.outside_execute_calls_count();
    assert_eq!(calls_count, 1);
    let calls = setup.rpc.outside_execute_calls_from(NON_CONTROLLER_ADDRESS);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].outside_execution, execute_outside);
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
        m.insert(CONTROLLER_ADDRESS, controller_calldata_response(CONTROLLER_ADDRESS));
        m
    };

    let setup = setup_test(cartridge_responses, HashMap::new()).await;

    let execute_outside = make_execute_outside();
    let params = json!([CONTROLLER_ADDRESS, execute_outside, ["0x0", "0x0"], null]);
    setup.call("cartridge_addExecuteOutsideTransaction", &params).await;

    // A deploy transaction should have been added to the pool.
    assert_eq!(setup.pool.size(), 1, "pool should contain 1 deploy transaction");

    // Inner service should have been called with the alternate method name.
    let calls_count = setup.rpc.outside_execute_calls_count();
    assert_eq!(calls_count, 1);
    let calls = setup.rpc.outside_execute_calls_from(CONTROLLER_ADDRESS);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].outside_execution, execute_outside);
    assert_eq!(calls[0].fee_source, None, "params shouldnt change");
    assert_eq!(calls[0].signature, vec![felt!("0x0"), felt!("0x0")], "params shouldnt change");
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

    let calls = setup.rpc.any_calls("starknet_getBlockNumber").expect("must be called");
    assert_eq!(calls.len(), 1, "starknet_getBlockNumber must be called once");

    let calls = setup.rpc.estimate_fee_calls();
    assert!(calls.is_empty());
    let calls_count = setup.rpc.outside_execute_calls_count();
    assert_eq!(calls_count, 0);

    // No Cartridge API requests should have been made because the method is not intercepted.
    assert!(!setup.mock_api_state.has_get_account_calldata_request(DEPLOYER_ADDRESS));
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
    let calls = setup.rpc.estimate_fee_calls();
    assert_eq!(calls.len(), 1);
    let calls_count = setup.rpc.outside_execute_calls_count();
    assert_eq!(calls_count, 0);
}

mod setup {
    use std::collections::HashMap;
    use std::future::Future;
    use std::sync::Arc;

    use ::cartridge::api::GetAccountCalldataResponse;
    use axum::extract::State;
    use axum::response::IntoResponse;
    use axum::routing::post;
    use axum::{Json, Router};
    use jsonrpsee::core::middleware::{Batch, Notification, RpcServiceT};
    use jsonrpsee::types::Request;
    use jsonrpsee::MethodResponse;
    use katana_chain_spec::ChainSpec;
    use katana_executor::blockifier::cache::ClassCache;
    use katana_executor::ExecutionFlags;
    use katana_gas_price_oracle::GasPriceOracle;
    use katana_pool::ordering::FiFo;
    use katana_pool::pool::Pool;
    use katana_pool::validation::NoopValidator;
    use katana_primitives::da::DataAvailabilityMode;
    use katana_primitives::execution::Call;
    use katana_primitives::fee::{AllResourceBoundsMapping, PriceUnit, ResourceBoundsMapping, Tip};
    use katana_primitives::transaction::{ExecutableTxWithHash, TxHash, TxNumber};
    use katana_primitives::{address, felt, ContractAddress, Felt};
    use katana_provider::api::state::StateProvider;
    use katana_provider::test_utils::test_provider;
    use katana_rpc_types::*;
    use katana_tasks::TaskManager;
    use parking_lot::Mutex;
    use serde::de::DeserializeOwned;
    use serde::Deserialize;
    use serde_json::json;
    use starknet::signers::SigningKey;
    use tokio::net::TcpListener;
    use tower::Layer;
    use url::Url;

    use crate::middleware::cartridge::{ControllerDeploymentLayer, ControllerDeploymentService};
    use crate::starknet::{
        PendingBlockProvider, RpcCache, StarknetApi, StarknetApiConfig, StarknetApiResult,
    };

    pub type TestPool =
        Pool<ExecutableTxWithHash, NoopValidator<ExecutableTxWithHash>, FiFo<ExecutableTxWithHash>>;

    /// An undeployed address that the mock API will recognize as a Controller.
    pub const CONTROLLER_ADDRESS: ContractAddress = address!("0xdead");
    /// An undeployed address that the mock API will NOT recognize as a Controller.
    pub const NON_CONTROLLER_ADDRESS: ContractAddress = address!("0xbeef");
    /// The deployer address — matches the genesis account at 0x1 in test_provider.
    pub const DEPLOYER_ADDRESS: ContractAddress = address!("0x1");

    type TestControllerDeploymentService = ControllerDeploymentService<
        MockRpcService,
        TestPool,
        MockPendingBlockProvider,
        katana_provider::DbProviderFactory,
    >;

    /// A complete test setup context.
    pub struct TestSetup {
        pub pool: TestPool,
        pub rpc: MockRpcService,
        pub mock_api_state: MockCartridgeApiState,

        controller_deployment_service: TestControllerDeploymentService,
    }

    impl TestSetup {
        /// Simulate a call to the [`ControllerDeploymentService`] with the given JSON-RPC request.
        ///
        /// The returned response is the response object returned directly from the Controller
        /// Deployment service.
        pub async fn call(&self, method: &str, params: &serde_json::Value) -> MethodResponse {
            let json = json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": method,
                "params": params
            });

            let json_str = json.to_string();
            let request: Request<'_> = serde_json::from_str(&json_str).unwrap();

            self.controller_deployment_service.call(request).await
        }
    }

    pub async fn setup_test(
        cartridge_api_responses: HashMap<ContractAddress, GetAccountCalldataResponse>,
        inner_rpc_responses: HashMap<&'static str, Vec<FeeEstimate>>,
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
            MockPendingBlockProvider,
            gas_oracle,
            config,
            storage,
            RpcCache::new(),
            ClassCache::new().unwrap(),
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

        TestSetup { controller_deployment_service: service, rpc: mock_rpc, mock_api_state, pool }
    }

    /// Builds a `serde_json::Value` response for the Cartridge API that represents
    /// a valid Controller account with some dummy constructor calldata.
    pub fn controller_calldata_response(address: ContractAddress) -> GetAccountCalldataResponse {
        GetAccountCalldataResponse {
            address,
            username: "testuser".to_string(),
            constructor_calldata: vec![
                felt!("0x24a9edbfa7082accfceabf6a92d7160086f346d622f28741bf1c651c412c9ab"),
                felt!("0x7465737475736572"),
                felt!("0x0"),
                felt!("0x2"),
                felt!("0x1"),
                felt!("0x2"),
            ],
        }
    }

    /// Creates a valid V3 invoke transaction for the given sender address.
    pub fn make_invoke_tx(sender_address: ContractAddress) -> BroadcastedTx {
        BroadcastedTx::Invoke(BroadcastedInvokeTx {
            sender_address,
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

    pub fn make_execute_outside() -> OutsideExecution {
        OutsideExecution::V2(OutsideExecutionV2 {
            caller: address!("0x414e595f43414c4c4552"),
            nonce: felt!("0x1"),
            execute_after: 0x0,
            execute_before: 0xffffffffffffffff,
            calls: vec![Call {
                contract_address: address!("0x1"),
                entry_point_selector: felt!("0x2"),
                calldata: vec![felt!("0x3")],
            }],
        })
    }

    async fn start_mock_cartridge_api(
        responses: HashMap<ContractAddress, GetAccountCalldataResponse>,
    ) -> (Url, MockCartridgeApiState) {
        let state = MockCartridgeApiState {
            predefined_responses: Arc::new(responses),
            get_account_calldata_request: Default::default(),
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

    #[derive(Debug, Deserialize)]
    struct GetAccountCalldataBody {
        address: ContractAddress,
    }

    async fn mock_cartridge_handler(
        State(state): State<MockCartridgeApiState>,
        Json(GetAccountCalldataBody { address }): Json<GetAccountCalldataBody>,
    ) -> impl IntoResponse {
        state.get_account_calldata_request.lock().push(address);

        if let Some(response) = state.predefined_responses.get(&address) {
            Json(response.clone()).into_response()
        } else {
            "Address not found".into_response()
        }
    }

    // ---- mock types ----

    /// A no-op pending block provider. All methods return `Ok(None)`, matching
    /// instant-mining mode behaviour.
    #[derive(Debug, Clone)]
    struct MockPendingBlockProvider;

    impl PendingBlockProvider for MockPendingBlockProvider {
        fn pending_state(&self) -> StarknetApiResult<Option<Box<dyn StateProvider>>> {
            Ok(None)
        }

        fn get_pending_state_update(&self) -> StarknetApiResult<Option<PreConfirmedStateUpdate>> {
            Ok(None)
        }

        fn get_pending_block_with_txs(
            &self,
        ) -> StarknetApiResult<Option<PreConfirmedBlockWithTxs>> {
            Ok(None)
        }

        fn get_pending_block_with_receipts(
            &self,
        ) -> StarknetApiResult<Option<PreConfirmedBlockWithReceipts>> {
            Ok(None)
        }

        fn get_pending_block_with_tx_hashes(
            &self,
        ) -> StarknetApiResult<Option<PreConfirmedBlockWithTxHashes>> {
            Ok(None)
        }

        fn get_pending_transaction(
            &self,
            _hash: TxHash,
        ) -> StarknetApiResult<Option<RpcTxWithHash>> {
            Ok(None)
        }

        // internally, the Controller deployment layer uses StarknetApi::add_invoke_tx_sync which
        // waits for the receipt to be available before returning. We return a random
        // receipt here to avoid blocking. The value is irrelevant because it's not being to
        // assert anything.
        fn get_pending_receipt(
            &self,
            hash: TxHash,
        ) -> StarknetApiResult<Option<TxReceiptWithBlockInfo>> {
            let _ = hash;
            Ok(Some(TxReceiptWithBlockInfo {
                transaction_hash: TxHash::ZERO,
                receipt: RpcTxReceipt::Invoke(RpcInvokeTxReceipt {
                    actual_fee: FeePayment { amount: Felt::ZERO, unit: PriceUnit::Fri },
                    finality_status: FinalityStatus::PreConfirmed,
                    messages_sent: Vec::new(),
                    events: Vec::new(),
                    execution_resources: ExecutionResources {
                        l1_data_gas: 0,
                        l1_gas: 0,
                        l2_gas: 0,
                    },
                    execution_result: ExecutionResult::Succeeded,
                }),
                block: ReceiptBlockInfo::PreConfirmed { block_number: 0 },
            }))
        }

        fn get_pending_trace(&self, _hash: TxHash) -> StarknetApiResult<Option<TxTrace>> {
            Ok(None)
        }

        fn get_pending_transaction_by_index(
            &self,
            _index: TxNumber,
        ) -> StarknetApiResult<Option<RpcTxWithHash>> {
            Ok(None)
        }
    }

    #[derive(Clone)]
    pub struct MockCartridgeApiState {
        /// Map from hex address (with "0x" prefix, lowercase) to the response JSON.
        predefined_responses: Arc<HashMap<ContractAddress, GetAccountCalldataResponse>>,

        /// Log of getAccountCalldata requests received for addresses that are KNOWN to be
        /// Controllers.
        get_account_calldata_request: Arc<Mutex<Vec<ContractAddress>>>,
    }

    impl MockCartridgeApiState {
        /// Returns whether the getAccountCalldata request has been made for the given address.
        pub fn has_get_account_calldata_request(&self, address: ContractAddress) -> bool {
            self.get_account_calldata_request.lock().contains(&address)
        }
    }

    #[derive(Clone, Debug, Default)]
    pub struct EstimateFeeRecordedCall {
        pub tx_count: usize,
    }

    #[derive(Clone, Debug)]
    pub struct OutsideExecuteRecordedCall {
        pub outside_execution: OutsideExecution,
        pub signature: Vec<Felt>,
        pub fee_source: Option<FeeSource>,
    }

    #[derive(Clone, Debug, Default)]
    pub struct AnyRecordedCall {}

    #[derive(Clone, Default)]
    pub struct MockRpcService {
        /// Pre-configured response JSON per method name.
        responses: Arc<HashMap<&'static str, Vec<FeeEstimate>>>,

        any_calls: Arc<Mutex<HashMap<String, Vec<AnyRecordedCall>>>>,
        estimate_fee_calls: Arc<Mutex<Vec<EstimateFeeRecordedCall>>>,
        outside_execute_calls: Arc<Mutex<Vec<(ContractAddress, OutsideExecuteRecordedCall)>>>,
    }

    impl MockRpcService {
        pub fn new(responses: HashMap<&'static str, Vec<FeeEstimate>>) -> Self {
            Self { responses: Arc::new(responses), ..Default::default() }
        }

        pub fn estimate_fee_calls(&self) -> Vec<EstimateFeeRecordedCall> {
            self.estimate_fee_calls.lock().clone()
        }

        pub fn outside_execute_calls_count(&self) -> usize {
            self.outside_execute_calls.lock().len()
        }

        pub fn outside_execute_calls_from(
            &self,
            sender_address: ContractAddress,
        ) -> Vec<OutsideExecuteRecordedCall> {
            self.outside_execute_calls
                .lock()
                .iter()
                .find(|(addr, _)| addr == &sender_address)
                .map(|(_, call)| call.clone())
                .into_iter()
                .collect()
        }

        pub fn any_calls(&self, method: &str) -> Option<Vec<AnyRecordedCall>> {
            self.any_calls.lock().get(method).cloned()
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

            match method.as_str() {
                "starknet_estimateFee" => {
                    // Parse the first param (array of txs) from the sequence params.
                    let params = request.params();
                    let mut seq = params.sequence();

                    let txs: Result<Vec<serde_json::Value>, _> = seq.next();
                    let tx_count = txs.ok().map(|v| v.len()).unwrap_or(0);

                    self.estimate_fee_calls.lock().push(EstimateFeeRecordedCall { tx_count });
                }

                "cartridge_addExecuteFromOutside" | "cartridge_addExecuteOutsideTransaction" => {
                    #[derive(Deserialize)]
                    struct NamedAddExecuteOutsideParams {
                        sender: ContractAddress,
                        outside_execution: OutsideExecution,
                        signature: Vec<Felt>,
                        fee_source: Option<FeeSource>,
                    }

                    fn parse_params<T: DeserializeOwned>(request: &Request<'_>) -> Option<T> {
                        match request.params().parse() {
                            Ok(params) => Some(params),
                            Err(..) => None,
                        }
                    }

                    let NamedAddExecuteOutsideParams {
                        sender,
                        signature,
                        fee_source,
                        outside_execution,
                    } = parse_params::<NamedAddExecuteOutsideParams>(&request).unwrap();

                    self.outside_execute_calls.lock().push((
                        sender,
                        OutsideExecuteRecordedCall { signature, fee_source, outside_execution },
                    ));
                }

                other => {
                    self.any_calls
                        .lock()
                        .entry(other.to_string())
                        .or_default()
                        .push(AnyRecordedCall {});
                }
            }

            let response = if let Some(resp) = self.responses.get(method.as_str()) {
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
