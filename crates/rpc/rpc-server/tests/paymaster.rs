//! Integration tests for PaymasterProxy.
//!
//! These tests verify that the PaymasterProxy correctly forwards requests to the upstream
//! server and returns the exact same responses.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use jsonrpsee::core::{async_trait, RpcResult};
use jsonrpsee::server::{ServerBuilder, ServerHandle};
use katana_rpc_api::paymaster::PaymasterApiServer;
use katana_rpc_server::paymaster::PaymasterProxy;
use paymaster_rpc::{
    BuildTransactionRequest, BuildTransactionResponse, ExecuteRawRequest, ExecuteRawResponse,
    ExecuteRequest, ExecuteResponse, TokenPrice,
};

#[tokio::test]
async fn test_health_proxy() {
    let tracker = RequestTracker::default();
    let (_handle, addr) = start_mock_server(tracker.clone()).await;

    let url = url::Url::parse(&format!("http://{}", addr)).unwrap();
    let proxy = PaymasterProxy::new(url, None).unwrap();

    // Call health through the proxy
    let result = PaymasterApiServer::health(&proxy).await.unwrap();

    // Verify the response matches
    assert!(result);

    // Verify the request was tracked
    let calls = tracker.health_calls.lock().unwrap();
    assert_eq!(*calls, 1);
}

#[tokio::test]
async fn test_is_available_proxy() {
    let tracker = RequestTracker::default();
    let (_handle, addr) = start_mock_server(tracker.clone()).await;

    let url = url::Url::parse(&format!("http://{}", addr)).unwrap();
    let proxy = PaymasterProxy::new(url, None).unwrap();

    // Call is_available through the proxy
    let result = PaymasterApiServer::is_available(&proxy).await.unwrap();

    // Verify the response matches
    assert!(result);

    // Verify the request was tracked
    let calls = tracker.is_available_calls.lock().unwrap();
    assert_eq!(*calls, 1);
}

#[tokio::test]
async fn test_build_transaction_proxy() {
    let tracker = RequestTracker::default();
    let (_handle, addr) = start_mock_server(tracker.clone()).await;

    let url = url::Url::parse(&format!("http://{}", addr)).unwrap();
    let proxy = PaymasterProxy::new(url, None).unwrap();

    // Create a test request from JSON to avoid type conflicts
    let request_json = r#"{
        "transaction": {
            "type": "invoke",
            "invoke": {
                "user_address": "0x5678",
                "calls": [{
                    "to": "0x1234",
                    "selector": "0xabc0",
                    "calldata": ["0x1", "0x2"]
                }]
            }
        },
        "parameters": {
            "version": "0x1",
            "fee_mode": {
                "mode": "default",
                "gas_token": "0x9999",
                "tip": "fast"
            }
        }
    }"#;

    let request: BuildTransactionRequest = serde_json::from_str(request_json).unwrap();

    // Serialize the original request for comparison
    let original_serialized = serde_json::to_string(&request).unwrap();

    // Call build_transaction through the proxy
    let result = PaymasterApiServer::build_transaction(&proxy, request).await.unwrap();

    // Verify the response matches the expected dummy response
    let expected = dummy_build_transaction_response();
    let result_serialized = serde_json::to_string(&result).unwrap();
    let expected_serialized = serde_json::to_string(&expected).unwrap();
    assert_eq!(result_serialized, expected_serialized);

    // Verify the request was proxied with the exact same data
    let requests = tracker.build_transaction_requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0], original_serialized);
}

#[tokio::test]
async fn test_execute_transaction_proxy() {
    let tracker = RequestTracker::default();
    let (_handle, addr) = start_mock_server(tracker.clone()).await;

    let url = url::Url::parse(&format!("http://{}", addr)).unwrap();
    let proxy = PaymasterProxy::new(url, None).unwrap();

    // Create a test request from JSON with valid TypedData to avoid type conflicts
    let request_json = format!(
        r#"{{
        "transaction": {{
            "type": "invoke",
            "invoke": {{
                "user_address": "0x1111",
                "typed_data": {},
                "signature": ["0x1a2b", "0x3c4d"]
            }}
        }},
        "parameters": {{
            "version": "0x1",
            "fee_mode": {{
                "mode": "sponsored",
                "tip": "slow"
            }},
            "time_bounds": {{
                "execute_after": 1000,
                "execute_before": 2000
            }}
        }}
    }}"#,
        valid_typed_data_json()
    );

    let request: ExecuteRequest = serde_json::from_str(&request_json).unwrap();

    // Serialize the original request for comparison
    let original_serialized = serde_json::to_string(&request).unwrap();

    // Call execute_transaction through the proxy
    let result = proxy.execute_transaction(request).await.unwrap();

    // Verify the response matches the expected dummy response
    let expected = dummy_execute_response();
    assert_eq!(serde_json::to_string(&result).unwrap(), serde_json::to_string(&expected).unwrap());

    // Verify the request was proxied with the exact same data
    let requests = tracker.execute_transaction_requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0], original_serialized);
}

#[tokio::test]
async fn test_execute_raw_transaction_proxy() {
    let tracker = RequestTracker::default();
    let (_handle, addr) = start_mock_server(tracker.clone()).await;

    let url = url::Url::parse(&format!("http://{}", addr)).unwrap();
    let proxy = PaymasterProxy::new(url, None).unwrap();

    // Create a test request from JSON to avoid type conflicts
    let request_json = r#"{
        "transaction": {
            "type": "raw_invoke",
            "invoke": {
                "user_address": "0x2222",
                "execute_from_outside_call": {
                    "to": "0x3333",
                    "selector": "0x4444",
                    "calldata": ["0x5555"]
                },
                "gas_token": "0x6666",
                "max_gas_token_amount": "0x7777"
            }
        },
        "parameters": {
            "version": "0x1",
            "fee_mode": {
                "mode": "default",
                "gas_token": "0x8888"
            }
        }
    }"#;

    let request: ExecuteRawRequest = serde_json::from_str(request_json).unwrap();

    // Serialize the original request for comparison
    let original_serialized = serde_json::to_string(&request).unwrap();

    // Call execute_raw_transaction through the proxy
    let result = proxy.execute_raw_transaction(request).await.unwrap();

    // Verify the response matches the expected dummy response
    let expected = dummy_execute_raw_response();
    assert_eq!(serde_json::to_string(&result).unwrap(), serde_json::to_string(&expected).unwrap());

    // Verify the request was proxied with the exact same data
    let requests = tracker.execute_raw_transaction_requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0], original_serialized);
}

#[tokio::test]
async fn test_get_supported_tokens_proxy() {
    let tracker = RequestTracker::default();
    let (_handle, addr) = start_mock_server(tracker.clone()).await;

    let url = url::Url::parse(&format!("http://{}", addr)).unwrap();
    let proxy = PaymasterProxy::new(url, None).unwrap();

    // Call get_supported_tokens through the proxy
    let result = proxy.get_supported_tokens().await.unwrap();

    // Verify the response matches the expected dummy response
    let expected = dummy_supported_tokens();
    assert_eq!(serde_json::to_string(&result).unwrap(), serde_json::to_string(&expected).unwrap());

    // Verify the request was tracked
    let calls = tracker.get_supported_tokens_calls.lock().unwrap();
    assert_eq!(*calls, 1);
}

#[tokio::test]
async fn test_proxy_with_api_key() {
    let tracker = RequestTracker::default();
    let (_handle, addr) = start_mock_server(tracker.clone()).await;

    let url = url::Url::parse(&format!("http://{}", addr)).unwrap();
    let api_key = "test-api-key-12345".to_string();
    let proxy = PaymasterProxy::new(url, Some(api_key)).unwrap();

    // Call health through the proxy with API key set
    let result = proxy.health().await.unwrap();

    // Verify the response matches (the mock doesn't validate the API key, just that it's set)
    assert!(result);

    // Verify the request was tracked
    let calls = tracker.health_calls.lock().unwrap();
    assert_eq!(*calls, 1);
}

#[tokio::test]
async fn test_multiple_requests_proxy() {
    let tracker = RequestTracker::default();
    let (_handle, addr) = start_mock_server(tracker.clone()).await;

    let url = url::Url::parse(&format!("http://{}", addr)).unwrap();
    let proxy = PaymasterProxy::new(url, None).unwrap();

    // Make multiple health calls
    for _ in 0..5 {
        let result = proxy.health().await.unwrap();
        assert!(result);
    }

    // Verify all requests were tracked
    let calls = tracker.health_calls.lock().unwrap();
    assert_eq!(*calls, 5);
}

#[tokio::test]
async fn test_request_data_integrity() {
    // This test verifies that complex nested request data is preserved exactly
    // when passing through the proxy
    let tracker = RequestTracker::default();
    let (_handle, addr) = start_mock_server(tracker.clone()).await;

    let url = url::Url::parse(&format!("http://{}", addr)).unwrap();
    let proxy = PaymasterProxy::new(url, None).unwrap();

    // Create a more complex request with various field types
    let request_json = r#"{
        "transaction": {
            "type": "deploy_and_invoke",
            "deployment": {
                "address": "0xdead",
                "class_hash": "0xbeef",
                "salt": "0xcafe",
                "calldata": ["0x1", "0x2", "0x3"],
                "sigdata": ["0xa", "0xb"],
                "version": 1
            },
            "invoke": {
                "user_address": "0xface",
                "calls": [
                    {"to": "0x100", "selector": "0x200", "calldata": ["0x300"]},
                    {"to": "0x400", "selector": "0x500", "calldata": ["0x600", "0x700"]}
                ]
            }
        },
        "parameters": {
            "version": "0x1",
            "fee_mode": {
                "mode": "default",
                "gas_token": "0xfee",
                "tip": "normal"
            },
            "time_bounds": {
                "execute_after": 12345,
                "execute_before": 67890
            }
        }
    }"#;

    let request: BuildTransactionRequest = serde_json::from_str(request_json).unwrap();
    let original_serialized = serde_json::to_string(&request).unwrap();

    // Call through proxy
    let _result = proxy.build_transaction(request).await.unwrap();

    // Verify the exact request was received by the mock server
    let requests = tracker.build_transaction_requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0], original_serialized);
}

/// Start a mock paymaster server and return the handle and address.
async fn start_mock_server(tracker: RequestTracker) -> (ServerHandle, SocketAddr) {
    let server = ServerBuilder::default().build("127.0.0.1:0").await.unwrap();

    let addr = server.local_addr().unwrap();
    let mock = MockPaymasterServer::new(tracker);
    let handle = server.start(mock.into_rpc());

    (handle, addr)
}

/// Tracks requests received by the mock server.
#[derive(Debug, Clone, Default)]
struct RequestTracker {
    health_calls: Arc<Mutex<u32>>,
    is_available_calls: Arc<Mutex<u32>>,
    build_transaction_requests: Arc<Mutex<Vec<String>>>,
    execute_transaction_requests: Arc<Mutex<Vec<String>>>,
    execute_raw_transaction_requests: Arc<Mutex<Vec<String>>>,
    get_supported_tokens_calls: Arc<Mutex<u32>>,
}

/// Mock PaymasterApiServer that tracks requests and returns predefined responses.
struct MockPaymasterServer {
    tracker: RequestTracker,
}

impl MockPaymasterServer {
    fn new(tracker: RequestTracker) -> Self {
        Self { tracker }
    }
}

#[async_trait]
impl PaymasterApiServer for MockPaymasterServer {
    async fn health(&self) -> RpcResult<bool> {
        let mut calls = self.tracker.health_calls.lock().unwrap();
        *calls += 1;
        Ok(true)
    }

    async fn is_available(&self) -> RpcResult<bool> {
        let mut calls = self.tracker.is_available_calls.lock().unwrap();
        *calls += 1;
        Ok(true)
    }

    async fn build_transaction(
        &self,
        req: BuildTransactionRequest,
    ) -> RpcResult<BuildTransactionResponse> {
        let serialized = serde_json::to_string(&req).unwrap();
        let mut requests = self.tracker.build_transaction_requests.lock().unwrap();
        requests.push(serialized);
        Ok(dummy_build_transaction_response())
    }

    async fn execute_transaction(&self, req: ExecuteRequest) -> RpcResult<ExecuteResponse> {
        let serialized = serde_json::to_string(&req).unwrap();
        let mut requests = self.tracker.execute_transaction_requests.lock().unwrap();
        requests.push(serialized);
        Ok(dummy_execute_response())
    }

    async fn execute_raw_transaction(
        &self,
        req: ExecuteRawRequest,
    ) -> RpcResult<ExecuteRawResponse> {
        let serialized = serde_json::to_string(&req).unwrap();
        let mut requests = self.tracker.execute_raw_transaction_requests.lock().unwrap();
        requests.push(serialized);
        Ok(dummy_execute_raw_response())
    }

    async fn get_supported_tokens(&self) -> RpcResult<Vec<TokenPrice>> {
        let mut calls = self.tracker.get_supported_tokens_calls.lock().unwrap();
        *calls += 1;
        Ok(dummy_supported_tokens())
    }
}

/// Valid TypedData JSON template with proper StarknetDomain type definition.
fn valid_typed_data_json() -> &'static str {
    r#"{
        "types": {
            "StarknetDomain": [
                { "name": "name", "type": "shortstring" },
                { "name": "version", "type": "shortstring" },
                { "name": "chainId", "type": "shortstring" },
                { "name": "revision", "type": "shortstring" }
            ],
            "TestMessage": [
                { "name": "value", "type": "felt" }
            ]
        },
        "primaryType": "TestMessage",
        "domain": {
            "name": "Test",
            "version": "1",
            "chainId": "SN_SEPOLIA",
            "revision": "1"
        },
        "message": {
            "value": "0x123"
        }
    }"#
}

/// Create a dummy BuildTransactionResponse JSON for testing.
fn dummy_build_transaction_response_json() -> String {
    format!(
        r#"{{
        "type": "invoke",
        "typed_data": {},
        "parameters": {{
            "version": "0x1",
            "fee_mode": {{
                "mode": "default",
                "gas_token": "0x1234"
            }}
        }},
        "fee": {{
            "gas_token_price_in_strk": "0x3e8",
            "estimated_fee_in_strk": "0x64",
            "estimated_fee_in_gas_token": "0xc8",
            "suggested_max_fee_in_strk": "0x96",
            "suggested_max_fee_in_gas_token": "0x12c"
        }}
    }}"#,
        valid_typed_data_json()
    )
}

/// Create a dummy ExecuteResponse JSON for testing.
fn dummy_execute_response_json() -> &'static str {
    r#"{
        "transaction_hash": "0xabcd",
        "tracking_id": "0x1234"
    }"#
}

/// Create a dummy ExecuteRawResponse JSON for testing.
fn dummy_execute_raw_response_json() -> &'static str {
    r#"{
        "transaction_hash": "0xdcba",
        "tracking_id": "0x4321"
    }"#
}

/// Create dummy supported tokens JSON for testing.
fn dummy_supported_tokens_json() -> &'static str {
    r#"[
        {"token_address": "0x1111", "decimals": 18, "price_in_strk": "0x1"},
        {"token_address": "0x2222", "decimals": 6, "price_in_strk": "0x2"}
    ]"#
}

fn dummy_build_transaction_response() -> BuildTransactionResponse {
    serde_json::from_str(&dummy_build_transaction_response_json()).unwrap()
}

fn dummy_execute_response() -> ExecuteResponse {
    serde_json::from_str(dummy_execute_response_json()).unwrap()
}

fn dummy_execute_raw_response() -> ExecuteRawResponse {
    serde_json::from_str(dummy_execute_raw_response_json()).unwrap()
}

fn dummy_supported_tokens() -> Vec<TokenPrice> {
    serde_json::from_str(dummy_supported_tokens_json()).unwrap()
}
