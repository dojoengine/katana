//! Reqwest-backed JSON-RPC HTTP client.
//!
//! This is a drop-in replacement for jsonrpsee's hyper-based HTTP client that routes requests
//! through [`reqwest`]. Unlike jsonrpsee's client, reqwest honors the standard proxy environment
//! variables (`HTTP_PROXY`, `HTTPS_PROXY`, `NO_PROXY`) — including CONNECT tunneling for HTTPS
//! targets — so any client built from this module transparently works behind an HTTP(S) proxy.
//!
//! Loopback targets (`localhost`, `127.0.0.1`, `::1`) always connect directly, even when a proxy
//! is configured. Local endpoints (sidecars, tests, a katana node on the same host) must stay
//! reachable without requiring users to also maintain a `NO_PROXY` entry.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use jsonrpsee::core::client::{BatchResponse, ClientT, Error};
use jsonrpsee::core::params::BatchRequestBuilder;
use jsonrpsee::core::traits::ToRpcParams;
use jsonrpsee::types::{
    ErrorObject, Id, InvalidRequestId, Response, ResponseSuccess, TwoPointZero,
};
use reqwest::header::{HeaderMap, CONTENT_TYPE};
use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::value::RawValue;
use url::Url;

const TEN_MB_SIZE_BYTES: u32 = 10 * 1024 * 1024;
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// A JSON-RPC HTTP client backed by [`reqwest`].
///
/// Implements jsonrpsee's [`ClientT`] so it can be used with any jsonrpsee-generated API client
/// trait. Build one with [`HttpClient::new`] or [`HttpClient::builder`].
#[derive(Debug, Clone)]
pub struct HttpClient {
    client: reqwest::Client,
    target: Url,
    request_id: Arc<AtomicU64>,
    max_response_size: u32,
}

impl HttpClient {
    /// Create a client with default settings connecting to the given target URL.
    pub fn new(target: Url) -> Result<Self, Error> {
        Self::builder().build(target)
    }

    /// Returns a builder for configuring the client.
    pub fn builder() -> HttpClientBuilder {
        HttpClientBuilder::default()
    }

    fn next_id(&self) -> u64 {
        self.request_id.fetch_add(1, Ordering::Relaxed)
    }

    async fn send(&self, body: String) -> Result<reqwest::Response, Error> {
        let response = self
            .client
            .post(self.target.clone())
            .header(CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(request_error)?;

        let status = response.status();
        if !status.is_success() {
            return Err(Error::Transport(TransportError::Rejected(status).into()));
        }

        Ok(response)
    }

    async fn send_and_read_body(&self, body: String) -> Result<Vec<u8>, Error> {
        let mut response = self.send(body).await?;
        let limit = self.max_response_size as usize;

        if response.content_length().is_some_and(|len| len > limit as u64) {
            return Err(Error::Transport(TransportError::ResponseTooLarge { limit }.into()));
        }

        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(request_error)? {
            if bytes.len() + chunk.len() > limit {
                return Err(Error::Transport(TransportError::ResponseTooLarge { limit }.into()));
            }
            bytes.extend_from_slice(&chunk);
        }

        Ok(bytes)
    }
}

/// Builder for [`HttpClient`].
#[derive(Debug, Clone)]
pub struct HttpClientBuilder {
    max_response_size: u32,
    request_timeout: Duration,
    headers: HeaderMap,
}

impl HttpClientBuilder {
    /// Set the maximum size (in bytes) of a response body. Default is 10 MiB.
    pub fn max_response_size(mut self, size: u32) -> Self {
        self.max_response_size = size;
        self
    }

    /// Set the timeout for each request. Default is 60 seconds.
    pub fn request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    /// Set headers to include in every request (e.g., authentication).
    pub fn set_headers(mut self, headers: HeaderMap) -> Self {
        self.headers = headers;
        self
    }

    /// Build the client connecting to the given target URL.
    pub fn build(self, target: Url) -> Result<HttpClient, Error> {
        let mut builder =
            reqwest::Client::builder().default_headers(self.headers).timeout(self.request_timeout);

        // Proxy env vars are read by reqwest when the client is built. Loopback targets must
        // remain reachable even when a proxy is configured, without requiring a NO_PROXY entry.
        if is_loopback(&target) {
            builder = builder.no_proxy();
        }

        let client = builder.build().map_err(|e| Error::Transport(e.into()))?;

        Ok(HttpClient {
            client,
            target,
            request_id: Arc::new(AtomicU64::new(0)),
            max_response_size: self.max_response_size,
        })
    }
}

impl Default for HttpClientBuilder {
    fn default() -> Self {
        Self {
            max_response_size: TEN_MB_SIZE_BYTES,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            headers: HeaderMap::new(),
        }
    }
}

impl ClientT for HttpClient {
    async fn notification<Params>(&self, method: &str, params: Params) -> Result<(), Error>
    where
        Params: ToRpcParams + Send,
    {
        let params = params.to_rpc_params()?;
        let notif = NotificationSer { jsonrpc: TwoPointZero, method, params: params.as_deref() };
        let body = serde_json::to_string(&notif).map_err(Error::ParseError)?;
        self.send(body).await?;
        Ok(())
    }

    async fn request<R, Params>(&self, method: &str, params: Params) -> Result<R, Error>
    where
        R: DeserializeOwned,
        Params: ToRpcParams + Send,
    {
        let id = self.next_id();
        let params = params.to_rpc_params()?;
        let request = RequestSer {
            jsonrpc: TwoPointZero,
            id: Id::Number(id),
            method,
            params: params.as_deref(),
        };

        let body = serde_json::to_string(&request).map_err(Error::ParseError)?;
        let bytes = self.send_and_read_body(body).await?;

        let response: Response<'_, Box<RawValue>> =
            serde_json::from_slice(&bytes).map_err(Error::ParseError)?;
        let success = ResponseSuccess::try_from(response)?;

        if success.id != Id::Number(id) {
            return Err(InvalidRequestId::NotPendingRequest(success.id.to_string()).into());
        }

        serde_json::from_str(success.result.get()).map_err(Error::ParseError)
    }

    async fn batch_request<'a, R>(
        &self,
        batch: BatchRequestBuilder<'a>,
    ) -> Result<BatchResponse<'a, R>, Error>
    where
        R: DeserializeOwned + fmt::Debug + 'a,
    {
        let batch = batch.build()?;
        let id_start = self.request_id.fetch_add(batch.len() as u64, Ordering::Relaxed);
        let id_range = id_start..id_start + batch.len() as u64;

        let requests: Vec<RequestSer<'_>> = batch
            .iter()
            .zip(id_range.clone())
            .map(|(entry, id)| RequestSer {
                jsonrpc: TwoPointZero,
                id: Id::Number(id),
                method: entry.0,
                params: entry.1.as_deref(),
            })
            .collect();

        let body = serde_json::to_string(&requests).map_err(Error::ParseError)?;
        let bytes = self.send_and_read_body(body).await?;

        let responses: Vec<Response<'_, Box<RawValue>>> =
            serde_json::from_slice(&bytes).map_err(Error::ParseError)?;

        // Responses may arrive in any order; match them back to their request by id.
        let mut entries = Vec::with_capacity(responses.len());
        for _ in 0..responses.len() {
            entries.push(Err(ErrorObject::borrowed(0, "", None)));
        }

        let mut successful_calls = 0;
        let mut failed_calls = 0;

        for response in responses {
            let id = response.id.try_parse_inner_as_number()?;

            let entry = match ResponseSuccess::try_from(response) {
                Ok(success) => {
                    let value =
                        serde_json::from_str(success.result.get()).map_err(Error::ParseError)?;
                    successful_calls += 1;
                    Ok(value)
                }
                Err(err) => {
                    failed_calls += 1;
                    Err(err)
                }
            };

            let slot = id
                .checked_sub(id_range.start)
                .and_then(|pos| usize::try_from(pos).ok())
                .and_then(|pos| entries.get_mut(pos));

            match slot {
                Some(elem) => *elem = entry,
                None => return Err(InvalidRequestId::NotPendingRequest(id.to_string()).into()),
            }
        }

        Ok(BatchResponse::new(successful_calls, entries, failed_calls))
    }
}

/// Serialization format of an outgoing JSON-RPC request.
#[derive(Serialize)]
struct RequestSer<'a> {
    jsonrpc: TwoPointZero,
    id: Id<'a>,
    method: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<&'a RawValue>,
}

/// Serialization format of an outgoing JSON-RPC notification.
#[derive(Serialize)]
struct NotificationSer<'a> {
    jsonrpc: TwoPointZero,
    method: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<&'a RawValue>,
}

#[derive(Debug, thiserror::Error)]
enum TransportError {
    #[error("request rejected: status code {0}")]
    Rejected(StatusCode),

    #[error("response body exceeded the size limit of {limit} bytes")]
    ResponseTooLarge { limit: usize },
}

fn request_error(err: reqwest::Error) -> Error {
    if err.is_timeout() {
        Error::RequestTimeout
    } else {
        Error::Transport(err.into())
    }
}

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use jsonrpsee::core::params::BatchRequestBuilder;
    use jsonrpsee::rpc_params;
    use jsonrpsee::server::{RpcModule, Server, ServerHandle};
    use jsonrpsee::types::ErrorObjectOwned;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;

    /// Starts a jsonrpsee server with a few test methods, returning its URL.
    async fn spawn_test_server() -> (Url, ServerHandle) {
        let server = Server::builder().build("127.0.0.1:0").await.unwrap();
        let addr = server.local_addr().unwrap();

        let mut module = RpcModule::new(());
        module.register_method("say_hello", |_, _, _| "hello").unwrap();
        module.register_method("echo", |params, _, _| params.one::<u64>().unwrap()).unwrap();
        module
            .register_method("fail", |_, _, _| -> Result<String, ErrorObjectOwned> {
                Err(ErrorObjectOwned::owned(-32099, "boom", None::<()>))
            })
            .unwrap();
        module.register_method("big", |_, _, _| "x".repeat(1024 * 1024)).unwrap();

        let handle = server.start(module);
        let url = Url::parse(&format!("http://{addr}")).unwrap();
        (url, handle)
    }

    #[tokio::test]
    async fn request_roundtrip() {
        let (url, _handle) = spawn_test_server().await;
        let client = HttpClient::new(url).unwrap();

        let result: String = client.request("say_hello", rpc_params![]).await.unwrap();
        assert_eq!(result, "hello");

        let result: u64 = client.request("echo", rpc_params![42u64]).await.unwrap();
        assert_eq!(result, 42);
    }

    #[tokio::test]
    async fn call_error_is_preserved() {
        let (url, _handle) = spawn_test_server().await;
        let client = HttpClient::new(url).unwrap();

        let err = client.request::<String, _>("fail", rpc_params![]).await.unwrap_err();
        match err {
            Error::Call(obj) => {
                assert_eq!(obj.code(), -32099);
                assert_eq!(obj.message(), "boom");
            }
            other => panic!("expected call error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn method_not_found_is_call_error() {
        let (url, _handle) = spawn_test_server().await;
        let client = HttpClient::new(url).unwrap();

        let err = client.request::<String, _>("no_such_method", rpc_params![]).await.unwrap_err();
        assert!(matches!(err, Error::Call(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn batch_request_preserves_order_and_counts() {
        let (url, _handle) = spawn_test_server().await;
        let client = HttpClient::new(url).unwrap();

        let mut batch = BatchRequestBuilder::new();
        batch.insert("echo", rpc_params![1u64]).unwrap();
        batch.insert("fail", rpc_params![]).unwrap();
        batch.insert("echo", rpc_params![3u64]).unwrap();

        let response = client.batch_request::<u64>(batch).await.unwrap();
        assert_eq!(response.num_successful_calls(), 2);
        assert_eq!(response.num_failed_calls(), 1);

        let entries: Vec<_> = response.iter().collect();
        assert_eq!(entries[0].as_ref().unwrap(), &1);
        assert_eq!(entries[1].as_ref().unwrap_err().code(), -32099);
        assert_eq!(entries[2].as_ref().unwrap(), &3);
    }

    #[tokio::test]
    async fn notification_does_not_error() {
        let (url, _handle) = spawn_test_server().await;
        let client = HttpClient::new(url).unwrap();
        client.notification("say_hello", rpc_params![]).await.unwrap();
    }

    #[tokio::test]
    async fn response_size_limit_is_enforced() {
        let (url, _handle) = spawn_test_server().await;
        let client = HttpClient::builder().max_response_size(1024).build(url).unwrap();

        let err = client.request::<String, _>("big", rpc_params![]).await.unwrap_err();
        match err {
            Error::Transport(inner) => {
                assert!(inner.to_string().contains("size limit"), "got {inner}");
            }
            other => panic!("expected transport error, got {other:?}"),
        }
    }

    /// A minimal HTTP forward proxy that answers any request with a canned JSON-RPC response,
    /// returning the request line it received.
    async fn spawn_mock_proxy() -> (std::net::SocketAddr, tokio::sync::oneshot::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel();

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();

            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            // Read until the end of headers, then drain the body based on content-length.
            let body_offset = loop {
                let n = stream.read(&mut chunk).await.unwrap();
                buf.extend_from_slice(&chunk[..n]);
                if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    break pos + 4;
                }
            };

            let head = String::from_utf8_lossy(&buf[..body_offset]).to_string();
            let content_length: usize = head
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(str::trim)
                        .map(String::from)
                })
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);

            while buf.len() < body_offset + content_length {
                let n = stream.read(&mut chunk).await.unwrap();
                buf.extend_from_slice(&chunk[..n]);
            }

            let body = r#"{"jsonrpc":"2.0","id":0,"result":"proxied"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: \
                 {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            stream.shutdown().await.ok();

            let request_line = head.lines().next().unwrap_or_default().to_string();
            tx.send(request_line).ok();
        });

        (addr, rx)
    }

    // NOTE: tests that mutate proxy env vars rely on cargo-nextest's process-per-test execution
    // model to avoid interfering with each other.

    #[tokio::test]
    async fn http_proxy_env_var_is_honored() {
        let (proxy_addr, request_line) = spawn_mock_proxy().await;
        std::env::set_var("HTTP_PROXY", format!("http://{proxy_addr}"));

        // The target host is not resolvable; the request can only succeed through the proxy.
        let url = Url::parse("http://katana-proxy-test.invalid/").unwrap();
        let client = HttpClient::new(url).unwrap();

        let result: String = client.request("say_hello", rpc_params![]).await.unwrap();
        assert_eq!(result, "proxied");

        // The proxy must have received the request in absolute-form.
        let request_line = request_line.await.unwrap();
        assert!(
            request_line.starts_with("POST http://katana-proxy-test.invalid/"),
            "got: {request_line}"
        );
    }

    #[tokio::test]
    async fn loopback_target_bypasses_proxy() {
        // Point the proxy env vars at an address that is guaranteed to refuse connections.
        let dead = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dead_addr = dead.local_addr().unwrap();
        drop(dead);
        std::env::set_var("HTTP_PROXY", format!("http://{dead_addr}"));
        std::env::set_var("HTTPS_PROXY", format!("http://{dead_addr}"));

        // A loopback target must connect directly and therefore still succeed.
        let (url, _handle) = spawn_test_server().await;
        let client = HttpClient::new(url).unwrap();

        let result: String = client.request("say_hello", rpc_params![]).await.unwrap();
        assert_eq!(result, "hello");
    }

    #[test]
    fn loopback_detection() {
        for url in ["http://localhost:5050", "http://127.0.0.1:5050", "http://[::1]:5050"] {
            assert!(is_loopback(&Url::parse(url).unwrap()), "{url}");
        }
        for url in ["http://example.com", "https://10.0.0.1:5050", "http://192.168.1.1"] {
            assert!(!is_loopback(&Url::parse(url).unwrap()), "{url}");
        }
    }
}
