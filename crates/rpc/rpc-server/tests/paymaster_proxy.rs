#![cfg(feature = "cartridge")]

use jsonrpsee::core::client::ClientT;
use jsonrpsee::core::params::ObjectParams;
use jsonrpsee::http_client::HttpClientBuilder;
use jsonrpsee::server::ServerBuilder;
use jsonrpsee::types::ErrorObjectOwned;
use jsonrpsee::RpcModule;
use katana_rpc_server::paymaster::PaymasterProxy;
use paymaster_rpc::{
    ExecuteRawRequest, ExecuteRawResponse, ExecuteRawTransactionParameters, ExecutionParameters,
    FeeMode, RawInvokeParameters,
};
use starknet_paymaster::core::types::{Call, Felt};
use url::Url;

fn sample_execute_raw_request() -> ExecuteRawRequest {
    ExecuteRawRequest {
        transaction: ExecuteRawTransactionParameters::RawInvoke {
            invoke: RawInvokeParameters {
                user_address: Felt::from(0x123_u128),
                execute_from_outside_call: Call {
                    to: Felt::from(0x456_u128),
                    selector: Felt::from(0x789_u128),
                    calldata: vec![Felt::from(0xabc_u128)],
                },
                gas_token: None,
                max_gas_token_amount: None,
            },
        },
        parameters: ExecutionParameters::V1 {
            fee_mode: FeeMode::Sponsored { tip: Default::default() },
            time_bounds: None,
        },
    }
}

fn execute_raw_params() -> ObjectParams {
    let request = sample_execute_raw_request();
    let mut params = ObjectParams::new();
    params.insert("transaction", request.transaction).expect("transaction params");
    params.insert("parameters", request.parameters).expect("parameters params");
    params
}

async fn spawn_server(
    module: RpcModule<()>,
) -> (std::net::SocketAddr, jsonrpsee::server::ServerHandle) {
    let server = ServerBuilder::default().build("127.0.0.1:0").await.expect("server to bind");
    let addr = server.local_addr().expect("server addr");
    let handle = server.start(module);
    (addr, handle)
}

#[tokio::test(flavor = "multi_thread")]
async fn paymaster_proxy_forwards_execute_raw() {
    let expected_hash = Felt::from(0xdead_u128);
    let expected_tracking = Felt::from(0xbeef_u128);

    let mut upstream = RpcModule::new(());
    upstream
        .register_async_method("paymaster_executeRawTransaction", move |params, _, _| async move {
            let request: ExecuteRawRequest = params.parse()?;
            match request.transaction {
                ExecuteRawTransactionParameters::RawInvoke { invoke } => {
                    assert_eq!(invoke.user_address, Felt::from(0x123_u128));
                    assert_eq!(invoke.execute_from_outside_call.to, Felt::from(0x456_u128));
                    assert_eq!(
                        invoke.execute_from_outside_call.calldata,
                        vec![Felt::from(0xabc_u128)]
                    );
                }
            }

            Ok::<_, ErrorObjectOwned>(ExecuteRawResponse {
                transaction_hash: expected_hash,
                tracking_id: expected_tracking,
            })
        })
        .expect("register paymaster handler");

    let (upstream_addr, _upstream_handle) = spawn_server(upstream).await;

    let proxy = PaymasterProxy::new(
        Url::parse(&format!("http://{upstream_addr}")).expect("valid upstream url"),
        None,
    );
    let proxy_module = proxy.module().expect("proxy module");
    let (proxy_addr, _proxy_handle) = spawn_server(proxy_module).await;

    let client =
        HttpClientBuilder::default().build(&format!("http://{proxy_addr}")).expect("proxy client");

    let response: ExecuteRawResponse = client
        .request("paymaster_executeRawTransaction", execute_raw_params())
        .await
        .expect("proxy response");

    assert_eq!(response.transaction_hash, expected_hash);
    assert_eq!(response.tracking_id, expected_tracking);
}

#[tokio::test(flavor = "multi_thread")]
async fn paymaster_proxy_passes_through_errors() {
    let mut upstream = RpcModule::new(());
    upstream
        .register_async_method("paymaster_executeRawTransaction", |_params, _, _| async move {
            Err::<ExecuteRawResponse, _>(ErrorObjectOwned::owned(
                -32099,
                "paymaster failure",
                Some("boom".to_string()),
            ))
        })
        .expect("register paymaster handler");

    let (upstream_addr, _upstream_handle) = spawn_server(upstream).await;

    let proxy = PaymasterProxy::new(
        Url::parse(&format!("http://{upstream_addr}")).expect("valid upstream url"),
        None,
    );
    let proxy_module = proxy.module().expect("proxy module");
    let (proxy_addr, _proxy_handle) = spawn_server(proxy_module).await;

    let client =
        HttpClientBuilder::default().build(&format!("http://{proxy_addr}")).expect("proxy client");

    let err = client
        .request::<ExecuteRawResponse, _>("paymaster_executeRawTransaction", execute_raw_params())
        .await
        .expect_err("expected error");

    let jsonrpsee::core::ClientError::Call(call_err) = err else {
        panic!("expected call error");
    };
    assert_eq!(call_err.code(), -32099);
    assert_eq!(call_err.message(), "paymaster failure");
}
