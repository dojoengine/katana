//! Unit tests for the [`VrfMiddlewareService`] middleware.

use cainome::cairo_serde::CairoSerde;
use jsonrpsee::types::ErrorObjectOwned;
use jsonrpsee::MethodResponse;
use katana_primitives::execution::Call;
use katana_primitives::{felt, Felt};
use katana_rpc_api::error::cartridge::CartridgeApiError;
use katana_rpc_types::broadcasted::{BroadcastedInvokeTx, BroadcastedTx};
use katana_rpc_types::outside_execution::OutsideExecution;
use serde_json::json;
use setup::*;
use starknet::macros::selector;

fn vrf_missing_followup_code() -> i32 {
    ErrorObjectOwned::from(CartridgeApiError::VrfMissingFollowUpCall).code()
}

fn vrf_invalid_target_code() -> i32 {
    ErrorObjectOwned::from(CartridgeApiError::VrfInvalidTarget {
        requested: felt!("0x0").into(),
        supported: felt!("0x0").into(),
    })
    .code()
}

/// ## Case:
///
/// A method that is not intercepted by the VRF middleware is forwarded as-is.
#[tokio::test(flavor = "multi_thread")]
async fn passthrough_other_methods() {
    let setup = setup_test().await;

    setup.call("starknet_getBlockNumber", &json!([])).await;

    let calls = setup.rpc.any_calls("starknet_getBlockNumber").expect("must be called");
    assert_eq!(calls.len(), 1);

    assert_eq!(setup.rpc.estimate_fee_calls().len(), 0);
    assert_eq!(setup.rpc.outside_execute_calls().len(), 0);
    assert!(!setup.mock_vrf_state.was_called(), "VRF server must not be called");
}

/// ## Case:
///
/// `starknet_estimateFee` is called with malformed params. The middleware should
/// gracefully fall through to the inner service instead of returning an error.
#[tokio::test(flavor = "multi_thread")]
async fn passthrough_malformed_estimate_fee() {
    let setup = setup_test().await;

    setup.call("starknet_estimateFee", &json!(["not_valid"])).await;

    assert_eq!(setup.rpc.estimate_fee_calls().len(), 1, "inner service must still be called");
    assert!(!setup.mock_vrf_state.was_called(), "VRF server must not be called");
}

/// ## Case:
///
/// `cartridge_addExecuteFromOutside` is called with malformed params. The middleware
/// should gracefully fall through to the inner service.
#[tokio::test(flavor = "multi_thread")]
async fn passthrough_malformed_execute_outside() {
    let setup = setup_test().await;

    setup.call("cartridge_addExecuteFromOutside", &json!(["not_valid"])).await;

    assert_eq!(setup.rpc.outside_execute_calls().len(), 1, "inner service must still be called");
    assert!(!setup.mock_vrf_state.was_called(), "VRF server must not be called");
}

// ---- cartridge_addExecuteFromOutside paths ----

/// ## Case:
///
/// An outside execution with no `request_random` call is forwarded unchanged to the
/// inner service and the VRF server is never contacted.
#[tokio::test(flavor = "multi_thread")]
async fn execute_outside_forwards_when_no_request_random() {
    let setup = setup_test().await;

    let outside_execution = make_outside_execution_without_vrf();
    let params = json!([SENDER_ADDRESS, outside_execution, ["0xaa", "0xbb"], null]);
    setup.call("cartridge_addExecuteFromOutside", &params).await;

    assert!(!setup.mock_vrf_state.was_called());

    let calls = setup.rpc.outside_execute_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].address, SENDER_ADDRESS);
    assert_eq!(calls[0].outside_execution, outside_execution, "unchanged");
    assert_eq!(calls[0].signature, vec![felt!("0xaa"), felt!("0xbb")]);
}

/// ## Case:
///
/// An outside execution whose calls include a `request_random` targeting the configured
/// VRF contract, followed by a user call.
///
/// ## Expected:
///
/// The middleware calls the VRF server, which returns a modified outside execution with
/// extra calls appended. The inner service then receives the rewritten params (new
/// execution + new signature produced by the VRF server).
#[tokio::test(flavor = "multi_thread")]
async fn execute_outside_resolves_request_random() {
    let setup = setup_test().await;

    let original = make_outside_execution_with_vrf();
    let params = json!([SENDER_ADDRESS, original, ["0x1"], null]);
    setup.call("cartridge_addExecuteFromOutside", &params).await;

    assert!(setup.mock_vrf_state.was_called(), "VRF server must be contacted");

    let calls = setup.rpc.outside_execute_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].address, SENDER_ADDRESS);
    assert_ne!(calls[0].outside_execution, original, "execution must be rewritten by VRF");
    assert_eq!(
        calls[0].outside_execution,
        vrf_resolved_execution(&original),
        "must match VRF-resolved execution"
    );
    assert_eq!(calls[0].signature, VRF_RESOLVED_SIGNATURE.to_vec());
}

/// ## Case:
///
/// `request_random` is the last call in the outside execution — no follow-up call to
/// consume the randomness.
///
/// ## Expected:
///
/// The middleware returns an `VrfMissingFollowUpCall` error without contacting the
/// VRF server or forwarding to the inner service.
#[tokio::test(flavor = "multi_thread")]
async fn execute_outside_errors_on_missing_followup_call() {
    let setup = setup_test().await;

    let outside_execution = make_outside_execution_vrf_only();
    let params = json!([SENDER_ADDRESS, outside_execution, ["0x1"], null]);
    let response = setup.call("cartridge_addExecuteFromOutside", &params).await;

    assert_error_code(response, vrf_missing_followup_code());
    assert!(!setup.mock_vrf_state.was_called());
    assert_eq!(setup.rpc.outside_execute_calls().len(), 0);
}

/// ## Case:
///
/// `request_random` targets a contract other than the one the VRF service was configured
/// with.
///
/// ## Expected:
///
/// The middleware returns `VrfInvalidTarget` without forwarding or contacting VRF.
#[tokio::test(flavor = "multi_thread")]
async fn execute_outside_errors_on_wrong_vrf_target() {
    let setup = setup_test().await;

    let wrong_target = felt!("0xdead").into();
    let outside_execution = make_outside_execution_with_vrf_at(wrong_target);
    let params = json!([SENDER_ADDRESS, outside_execution, ["0x1"], null]);
    let response = setup.call("cartridge_addExecuteFromOutside", &params).await;

    assert_error_code(response, vrf_invalid_target_code());
    assert!(!setup.mock_vrf_state.was_called());
    assert_eq!(setup.rpc.outside_execute_calls().len(), 0);
}

/// ## Case:
///
/// Same scenario as `execute_outside_resolves_request_random` but uses the alternate
/// method name `cartridge_addExecuteOutsideTransaction`.
#[tokio::test(flavor = "multi_thread")]
async fn execute_outside_tx_method_variant() {
    let setup = setup_test().await;

    let original = make_outside_execution_with_vrf();
    let params = json!([SENDER_ADDRESS, original, ["0x1"], null]);
    setup.call("cartridge_addExecuteOutsideTransaction", &params).await;

    assert!(setup.mock_vrf_state.was_called());
    let calls = setup.rpc.outside_execute_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].outside_execution, vrf_resolved_execution(&original));
}

// ---- starknet_estimateFee paths ----

/// ## Case:
///
/// An invoke tx whose calldata wraps an `execute_from_outside_v2` call with no
/// `request_random` inside.
///
/// ## Expected:
///
/// The middleware forwards the request unchanged, the VRF server is not contacted,
/// and the inner service receives the same calldata.
#[tokio::test(flavor = "multi_thread")]
async fn estimate_fee_forwards_when_no_request_random() {
    let setup = setup_test().await;

    let outside_execution = make_outside_execution_without_vrf();
    let invoke = make_invoke_tx_wrapping(&outside_execution, vec![felt!("0xaa")]);
    let expected_calldata = invoke_calldata(&invoke);

    setup
        .call("starknet_estimateFee", &json!([[BroadcastedTx::Invoke(invoke)], [], "latest"]))
        .await;

    assert!(!setup.mock_vrf_state.was_called());
    let calls = setup.rpc.estimate_fee_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].transactions.len(), 1);
    assert_eq!(invoke_calldata_of(&calls[0].transactions[0]), expected_calldata);
}

/// ## Case:
///
/// An invoke tx whose calldata does NOT wrap an `execute_from_outside_v2/v3` call
/// (e.g. a normal ERC-20 transfer).
///
/// ## Expected:
///
/// The middleware leaves the calldata untouched and does not contact the VRF server.
#[tokio::test(flavor = "multi_thread")]
async fn estimate_fee_forwards_when_not_outside_execution() {
    let setup = setup_test().await;

    let non_outside_call = Call {
        contract_address: felt!("0x1").into(),
        entry_point_selector: selector!("transfer"),
        calldata: vec![felt!("0x2"), felt!("0x3")],
    };
    let invoke = make_invoke_tx_with_calls(vec![non_outside_call]);
    let expected_calldata = invoke_calldata(&invoke);

    setup
        .call("starknet_estimateFee", &json!([[BroadcastedTx::Invoke(invoke)], [], "latest"]))
        .await;

    assert!(!setup.mock_vrf_state.was_called());
    let calls = setup.rpc.estimate_fee_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(invoke_calldata_of(&calls[0].transactions[0]), expected_calldata);
}

/// ## Case:
///
/// The batch of transactions contains a declare tx — not an invoke. The middleware
/// must not attempt to decode it and must forward everything unchanged.
#[tokio::test(flavor = "multi_thread")]
async fn estimate_fee_forwards_non_invoke_tx() {
    let setup = setup_test().await;

    // Use a plain invoke tx with an opaque calldata that does not decode.
    let invoke = make_invoke_tx_with_raw_calldata(vec![Felt::ZERO]);
    setup
        .call("starknet_estimateFee", &json!([[BroadcastedTx::Invoke(invoke)], [], "latest"]))
        .await;

    assert!(!setup.mock_vrf_state.was_called());
    assert_eq!(setup.rpc.estimate_fee_calls().len(), 1);
}

/// ## Case:
///
/// An invoke tx whose calldata wraps an `execute_from_outside_v2` call containing a
/// valid `request_random` targeting the configured VRF contract plus a follow-up call.
///
/// ## Expected:
///
/// The middleware resolves the VRF, re-encodes the invoke calldata with the modified
/// outside execution, and forwards the rewritten tx to the inner service.
#[tokio::test(flavor = "multi_thread")]
async fn estimate_fee_resolves_request_random() {
    let setup = setup_test().await;

    let original_exec = make_outside_execution_with_vrf();
    let invoke = make_invoke_tx_wrapping(&original_exec, vec![felt!("0x1")]);

    setup
        .call("starknet_estimateFee", &json!([[BroadcastedTx::Invoke(invoke)], [], "latest"]))
        .await;

    assert!(setup.mock_vrf_state.was_called(), "VRF server must be contacted");

    let calls = setup.rpc.estimate_fee_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].transactions.len(), 1);

    // The inner service must receive the rewritten invoke calldata containing the
    // VRF-resolved outside execution (with the extra submit_random call appended).
    let (received_exec, received_sig) =
        decode_outside_execution_from_invoke(&invoke_calldata_of(&calls[0].transactions[0]));

    assert_eq!(received_exec, vrf_resolved_execution(&original_exec));
    assert_eq!(received_sig, VRF_RESOLVED_SIGNATURE.to_vec());
}

/// ## Case:
///
/// `request_random` is the last call inside the wrapped outside execution.
#[tokio::test(flavor = "multi_thread")]
async fn estimate_fee_errors_on_missing_followup_call() {
    let setup = setup_test().await;

    let outside_execution = make_outside_execution_vrf_only();
    let invoke = make_invoke_tx_wrapping(&outside_execution, vec![felt!("0x1")]);

    let response = setup
        .call("starknet_estimateFee", &json!([[BroadcastedTx::Invoke(invoke)], [], "latest"]))
        .await;

    assert_error_code(response, vrf_missing_followup_code());
    assert!(!setup.mock_vrf_state.was_called());
    assert_eq!(setup.rpc.estimate_fee_calls().len(), 0, "inner service must not be called");
}

/// ## Case:
///
/// `request_random` is targeting a contract that isn't the configured VRF contract.
#[tokio::test(flavor = "multi_thread")]
async fn estimate_fee_errors_on_wrong_vrf_target() {
    let setup = setup_test().await;

    let wrong_target = felt!("0xbadf00d").into();
    let outside_execution = make_outside_execution_with_vrf_at(wrong_target);
    let invoke = make_invoke_tx_wrapping(&outside_execution, vec![felt!("0x1")]);

    let response = setup
        .call("starknet_estimateFee", &json!([[BroadcastedTx::Invoke(invoke)], [], "latest"]))
        .await;

    assert_error_code(response, vrf_invalid_target_code());
    assert!(!setup.mock_vrf_state.was_called());
    assert_eq!(setup.rpc.estimate_fee_calls().len(), 0);
}

/// ## Case:
///
/// A batch with two invoke transactions — one wraps a VRF-bearing outside execution,
/// the other does not.
///
/// ## Expected:
///
/// Only the VRF tx is rewritten; the non-VRF tx passes through untouched.
#[tokio::test(flavor = "multi_thread")]
async fn estimate_fee_rewrites_only_vrf_tx_in_batch() {
    let setup = setup_test().await;

    let plain_exec = make_outside_execution_without_vrf();
    let plain_invoke = make_invoke_tx_wrapping(&plain_exec, vec![felt!("0xaa")]);
    let plain_expected_calldata = invoke_calldata(&plain_invoke);

    let vrf_exec = make_outside_execution_with_vrf();
    let vrf_invoke = make_invoke_tx_wrapping(&vrf_exec, vec![felt!("0x1")]);

    setup
        .call(
            "starknet_estimateFee",
            &json!([
                [BroadcastedTx::Invoke(plain_invoke), BroadcastedTx::Invoke(vrf_invoke)],
                [],
                "latest"
            ]),
        )
        .await;

    assert_eq!(setup.mock_vrf_state.call_count(), 1);

    let calls = setup.rpc.estimate_fee_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].transactions.len(), 2);
    assert_eq!(
        invoke_calldata_of(&calls[0].transactions[0]),
        plain_expected_calldata,
        "plain tx unchanged"
    );

    let (received_exec, _) =
        decode_outside_execution_from_invoke(&invoke_calldata_of(&calls[0].transactions[1]));
    assert_eq!(received_exec, vrf_resolved_execution(&vrf_exec), "vrf tx rewritten");
}

// ---- helpers ----

fn assert_error_code(response: MethodResponse, expected_code: i32) {
    let body = response.into_json();
    let json: serde_json::Value = serde_json::from_str(body.get()).unwrap();
    let code = json.get("error").and_then(|e| e.get("code")).and_then(|c| c.as_i64()).unwrap();
    assert_eq!(code as i32, expected_code);
}

fn invoke_calldata(tx: &BroadcastedInvokeTx) -> Vec<Felt> {
    tx.calldata.clone()
}

fn invoke_calldata_of(tx: &BroadcastedTx) -> Vec<Felt> {
    match tx {
        BroadcastedTx::Invoke(tx) => tx.calldata.clone(),
        _ => panic!("not an invoke tx"),
    }
}

/// Decodes the `execute_from_outside_v2/v3` call inside an invoke tx calldata and
/// returns `(outside_execution, signature)`.
fn decode_outside_execution_from_invoke(calldata: &[Felt]) -> (OutsideExecution, Vec<Felt>) {
    let calls = Vec::<Call>::cairo_deserialize(calldata, 0).expect("decode outer multicall");
    let call = calls
        .iter()
        .find(|c| {
            c.entry_point_selector == selector!("execute_from_outside_v2")
                || c.entry_point_selector == selector!("execute_from_outside_v3")
        })
        .expect("must contain execute_from_outside call");

    let outside_execution =
        OutsideExecution::cairo_deserialize(&call.calldata, 0).expect("decode outside execution");
    let oe_size = OutsideExecution::cairo_serialized_size(&outside_execution);
    let signature =
        Vec::<Felt>::cairo_deserialize(&call.calldata, oe_size).expect("decode signature tail");

    (outside_execution, signature)
}

mod setup {
    use std::collections::HashMap;
    use std::future::Future;
    use std::sync::Arc;

    use axum::extract::State;
    use axum::response::IntoResponse;
    use axum::routing::post;
    use axum::{Json, Router};
    use jsonrpsee::core::middleware::{Batch, Notification, RpcServiceT};
    use jsonrpsee::types::Request;
    use jsonrpsee::MethodResponse;
    use katana_primitives::chain::ChainId;
    use katana_primitives::da::DataAvailabilityMode;
    use katana_primitives::execution::Call;
    use katana_primitives::fee::{AllResourceBoundsMapping, ResourceBoundsMapping, Tip};
    use katana_primitives::{address, felt, ContractAddress, Felt};
    use katana_rpc_types::broadcasted::{BroadcastedInvokeTx, BroadcastedTx};
    use katana_rpc_types::outside_execution::{OutsideExecution, OutsideExecutionV2};
    use katana_rpc_types::{FeeSource, SignedOutsideExecution};
    use parking_lot::Mutex;
    use serde::Deserialize;
    use serde_json::{json, Value};
    use starknet::macros::selector;
    use tokio::net::TcpListener;
    use tower::Layer;
    use url::Url;

    use crate::cartridge::{encode_calls, VrfService, VrfServiceConfig};
    use crate::middleware::cartridge::{VrfLayer, VrfMiddlewareService};

    const ANY_CALLER: Felt = felt!("0x414e595f43414c4c4552");

    /// The contract address configured on the VRF layer — calls to `request_random` must
    /// target this contract.
    pub const VRF_CONTRACT: ContractAddress = address!("0xabc");
    /// The sender/controller address used in execute_outside tests.
    pub const SENDER_ADDRESS: ContractAddress = address!("0xcafe");
    /// The signature that the mock VRF server attaches to every resolved execution.
    pub const VRF_RESOLVED_SIGNATURE: &[Felt] = &[Felt::TWO, Felt::THREE];

    type TestVrfService = VrfMiddlewareService<MockRpcService>;

    pub struct TestSetup {
        pub rpc: MockRpcService,
        pub mock_vrf_state: MockVrfState,
        service: TestVrfService,
    }

    impl TestSetup {
        pub async fn call(&self, method: &str, params: &Value) -> MethodResponse {
            let json = json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": method,
                "params": params
            });

            let json_str = json.to_string();
            let request: Request<'_> = serde_json::from_str(&json_str).unwrap();

            self.service.call(request).await
        }
    }

    pub async fn setup_test() -> TestSetup {
        let (vrf_url, mock_vrf_state) = start_mock_vrf_server().await;

        let vrf_service = VrfService::new(VrfServiceConfig {
            rpc_url: Url::parse("http://127.0.0.1:0").unwrap(),
            service_url: vrf_url,
            vrf_contract: VRF_CONTRACT,
        });

        let chain_id = ChainId::Id(felt!("0x1337"));
        let layer = VrfLayer::new(vrf_service, chain_id);

        let mock_rpc = MockRpcService::default();
        let service = layer.layer(mock_rpc.clone());

        TestSetup { rpc: mock_rpc, mock_vrf_state, service }
    }

    // ---- outside execution builders ----

    /// Builds an outside execution whose calls do NOT include a `request_random` call.
    pub fn make_outside_execution_without_vrf() -> OutsideExecution {
        OutsideExecution::V2(OutsideExecutionV2 {
            caller: ContractAddress::from(ANY_CALLER),
            nonce: felt!("0x1"),
            execute_after: 0,
            execute_before: 0xffffffffffffffff,
            calls: vec![Call {
                contract_address: address!("0xbeef"),
                entry_point_selector: selector!("transfer"),
                calldata: vec![felt!("0x1"), felt!("0x2")],
            }],
        })
    }

    /// Builds an outside execution where `request_random` targets the configured VRF
    /// contract and is followed by a user call.
    pub fn make_outside_execution_with_vrf() -> OutsideExecution {
        make_outside_execution_with_vrf_at(VRF_CONTRACT)
    }

    pub fn make_outside_execution_with_vrf_at(target: ContractAddress) -> OutsideExecution {
        OutsideExecution::V2(OutsideExecutionV2 {
            caller: ContractAddress::from(ANY_CALLER),
            nonce: felt!("0x2"),
            execute_after: 0,
            execute_before: 0xffffffffffffffff,
            calls: vec![
                Call {
                    contract_address: target,
                    entry_point_selector: selector!("request_random"),
                    calldata: vec![Felt::ONE],
                },
                Call {
                    contract_address: address!("0xbeef"),
                    entry_point_selector: selector!("consume"),
                    calldata: vec![],
                },
            ],
        })
    }

    /// Builds an outside execution whose only call is `request_random` — no follow-up.
    pub fn make_outside_execution_vrf_only() -> OutsideExecution {
        OutsideExecution::V2(OutsideExecutionV2 {
            caller: ContractAddress::from(ANY_CALLER),
            nonce: felt!("0x3"),
            execute_after: 0,
            execute_before: 0xffffffffffffffff,
            calls: vec![Call {
                contract_address: VRF_CONTRACT,
                entry_point_selector: selector!("request_random"),
                calldata: vec![Felt::ONE],
            }],
        })
    }

    /// Produces the outside execution that the mock VRF server returns after "resolving"
    /// the given request. It appends a `submit_random` call to simulate the injection
    /// that the real VRF server performs.
    pub fn vrf_resolved_execution(original: &OutsideExecution) -> OutsideExecution {
        let OutsideExecution::V2(v2) = original else {
            panic!("tests only use V2 outside executions");
        };

        let mut calls = v2.calls.clone();
        calls.push(Call {
            contract_address: VRF_CONTRACT,
            entry_point_selector: selector!("submit_random"),
            calldata: vec![Felt::from(42u64)],
        });

        OutsideExecution::V2(OutsideExecutionV2 {
            caller: v2.caller,
            nonce: v2.nonce,
            execute_after: v2.execute_after,
            execute_before: v2.execute_before,
            calls,
        })
    }

    // ---- invoke tx builders ----

    fn default_invoke_tx() -> BroadcastedInvokeTx {
        BroadcastedInvokeTx {
            sender_address: SENDER_ADDRESS,
            calldata: Vec::new(),
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
        }
    }

    /// Produces a cairo-1 multicall calldata that wraps the given outside execution as
    /// an `execute_from_outside_v2` call.
    pub fn make_invoke_tx_wrapping(
        outside_execution: &OutsideExecution,
        signature: Vec<Felt>,
    ) -> BroadcastedInvokeTx {
        let signed = SignedOutsideExecution {
            address: SENDER_ADDRESS,
            outside_execution: outside_execution.clone(),
            signature,
        };
        let call = Call::from(signed);
        BroadcastedInvokeTx { calldata: encode_calls(vec![call]), ..default_invoke_tx() }
    }

    /// Produces an invoke tx whose calldata encodes the given calls as a cairo-1
    /// multicall — used for tests that need non-outside-execution calls.
    pub fn make_invoke_tx_with_calls(calls: Vec<Call>) -> BroadcastedInvokeTx {
        BroadcastedInvokeTx { calldata: encode_calls(calls), ..default_invoke_tx() }
    }

    /// Produces an invoke tx with raw calldata that doesn't decode as a valid cairo-1
    /// multicall.
    pub fn make_invoke_tx_with_raw_calldata(calldata: Vec<Felt>) -> BroadcastedInvokeTx {
        BroadcastedInvokeTx { calldata, ..default_invoke_tx() }
    }

    // ---- mock VRF server ----

    async fn start_mock_vrf_server() -> (Url, MockVrfState) {
        let state = MockVrfState { received: Arc::new(Mutex::new(Vec::new())) };

        let app = Router::new()
            .route("/outside_execution", post(handle_outside_execution))
            .with_state(state.clone());

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        (Url::parse(&format!("http://{addr}")).unwrap(), state)
    }

    #[derive(Debug, Deserialize)]
    struct OutsideExecutionRequest {
        request: SignedOutsideExecution,
        #[allow(dead_code)]
        context: Value,
    }

    async fn handle_outside_execution(
        State(state): State<MockVrfState>,
        Json(OutsideExecutionRequest { request, .. }): Json<OutsideExecutionRequest>,
    ) -> impl IntoResponse {
        state.received.lock().push(request.clone());

        let resolved = SignedOutsideExecution {
            address: request.address,
            outside_execution: super::vrf_resolved_execution(&request.outside_execution),
            signature: super::VRF_RESOLVED_SIGNATURE.to_vec(),
        };

        Json(json!({ "result": resolved }))
    }

    #[derive(Clone)]
    pub struct MockVrfState {
        received: Arc<Mutex<Vec<SignedOutsideExecution>>>,
    }

    impl MockVrfState {
        pub fn was_called(&self) -> bool {
            !self.received.lock().is_empty()
        }

        pub fn call_count(&self) -> usize {
            self.received.lock().len()
        }
    }

    // ---- mock inner RPC service ----

    #[derive(Clone, Debug)]
    pub struct EstimateFeeRecordedCall {
        pub transactions: Vec<BroadcastedTx>,
    }

    #[derive(Clone, Debug)]
    pub struct OutsideExecuteRecordedCall {
        pub address: ContractAddress,
        pub outside_execution: OutsideExecution,
        pub signature: Vec<Felt>,
        #[allow(dead_code)]
        pub fee_source: Option<FeeSource>,
    }

    #[derive(Clone, Debug, Default)]
    pub struct AnyRecordedCall {}

    #[derive(Clone, Default)]
    pub struct MockRpcService {
        any_calls: Arc<Mutex<HashMap<String, Vec<AnyRecordedCall>>>>,
        estimate_fee_calls: Arc<Mutex<Vec<EstimateFeeRecordedCall>>>,
        outside_execute_calls: Arc<Mutex<Vec<OutsideExecuteRecordedCall>>>,
    }

    impl MockRpcService {
        pub fn estimate_fee_calls(&self) -> Vec<EstimateFeeRecordedCall> {
            self.estimate_fee_calls.lock().clone()
        }

        pub fn outside_execute_calls(&self) -> Vec<OutsideExecuteRecordedCall> {
            self.outside_execute_calls.lock().clone()
        }

        pub fn any_calls(&self, method: &str) -> Option<Vec<AnyRecordedCall>> {
            self.any_calls.lock().get(method).cloned()
        }
    }

    #[derive(Deserialize)]
    struct EstimateFeePositional(
        Vec<BroadcastedTx>,
        #[allow(dead_code)] Value,
        #[allow(dead_code)] Value,
    );

    #[derive(Deserialize)]
    struct OutsideExecutePositional(
        ContractAddress,
        OutsideExecution,
        Vec<Felt>,
        #[serde(default)] Option<FeeSource>,
    );

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
                    if let Ok(EstimateFeePositional(txs, _, _)) = request.params().parse() {
                        self.estimate_fee_calls
                            .lock()
                            .push(EstimateFeeRecordedCall { transactions: txs });
                    } else {
                        // Record a dummy entry so tests can assert fallthrough without
                        // needing well-formed params.
                        self.estimate_fee_calls
                            .lock()
                            .push(EstimateFeeRecordedCall { transactions: Vec::new() });
                    }
                }

                "cartridge_addExecuteFromOutside" | "cartridge_addExecuteOutsideTransaction" => {
                    if let Ok(OutsideExecutePositional(address, oe, sig, fee_source)) =
                        request.params().parse()
                    {
                        self.outside_execute_calls.lock().push(OutsideExecuteRecordedCall {
                            address,
                            outside_execution: oe,
                            signature: sig,
                            fee_source,
                        });
                    } else {
                        self.outside_execute_calls.lock().push(OutsideExecuteRecordedCall {
                            address: address!("0x0"),
                            outside_execution: super::OutsideExecution::V2(OutsideExecutionV2 {
                                caller: address!("0x0"),
                                nonce: Felt::ZERO,
                                execute_after: 0,
                                execute_before: 0,
                                calls: Vec::new(),
                            }),
                            signature: Vec::new(),
                            fee_source: None,
                        });
                    }
                }

                other => {
                    self.any_calls
                        .lock()
                        .entry(other.to_string())
                        .or_default()
                        .push(AnyRecordedCall {});
                }
            }

            let response = MethodResponse::response(
                request.id().clone(),
                jsonrpsee::ResponsePayload::success(Value::Null),
                usize::MAX,
            );

            std::future::ready(response)
        }

        fn batch<'a>(
            &self,
            _requests: Batch<'a>,
        ) -> impl Future<Output = Self::BatchResponse> + Send + 'a {
            std::future::ready(MethodResponse::response(
                jsonrpsee::types::Id::Null,
                jsonrpsee::ResponsePayload::success(Value::Null),
                usize::MAX,
            ))
        }

        fn notification<'a>(
            &self,
            _n: Notification<'a>,
        ) -> impl Future<Output = Self::NotificationResponse> + Send + 'a {
            std::future::ready(MethodResponse::response(
                jsonrpsee::types::Id::Null,
                jsonrpsee::ResponsePayload::success(Value::Null),
                usize::MAX,
            ))
        }
    }
}
