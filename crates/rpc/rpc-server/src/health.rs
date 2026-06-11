use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use http::header::{CONNECTION, UPGRADE};
use jsonrpsee::core::BoxError;
use jsonrpsee::server::middleware::http::ProxyGetRequestLayer;
use jsonrpsee::server::{HttpRequest, HttpResponse};
use jsonrpsee::{Methods, ResponsePayload, RpcModule};
use serde_json::json;
use tower::{Layer, Service};

/// Simple health check endpoint.
#[derive(Debug)]
pub struct HealthCheck;

impl HealthCheck {
    const METHOD: &'static str = "health";
    const PROXY_PATH: &'static str = "/";

    pub(crate) fn proxy() -> WsPassthroughProxyLayer {
        let inner = Self::proxy_with_path(Self::PROXY_PATH);
        WsPassthroughProxyLayer { inner }
    }

    fn proxy_with_path(path: &str) -> ProxyGetRequestLayer {
        ProxyGetRequestLayer::new([(path, Self::METHOD)]).expect("path starts with /")
    }
}

impl From<HealthCheck> for Methods {
    fn from(_: HealthCheck) -> Self {
        let mut module = RpcModule::new(());

        module
            .register_method(HealthCheck::METHOD, |_, _, _| {
                ResponsePayload::success(json!({ "health": true }))
            })
            .unwrap();

        module.into()
    }
}

/// A [`Layer`] that wraps [`ProxyGetRequestLayer`] but passes WebSocket upgrade requests through
/// untouched so that jsonrpsee's built-in WebSocket handler can process the upgrade handshake.
///
/// Without this, the `ProxyGetRequestLayer` on `/` intercepts ALL GET requests — including
/// WebSocket upgrades — converting them into JSON-RPC POST calls, which breaks the WS handshake.
#[derive(Debug, Clone)]
pub struct WsPassthroughProxyLayer {
    inner: ProxyGetRequestLayer,
}

impl<S: Clone> Layer<S> for WsPassthroughProxyLayer {
    type Service = WsPassthroughProxy<S>;

    fn layer(&self, service: S) -> Self::Service {
        WsPassthroughProxy { proxy: self.inner.layer(service.clone()), inner: service }
    }
}

/// Service created by [`WsPassthroughProxyLayer`].
///
/// For WebSocket upgrade requests, forwards directly to the inner service (bypassing the proxy).
/// For all other requests, delegates to the health-check [`ProxyGetRequest`] proxy.
#[derive(Debug, Clone)]
pub struct WsPassthroughProxy<S> {
    proxy: jsonrpsee::server::middleware::http::ProxyGetRequest<S>,
    inner: S,
}

/// Returns `true` if the request carries WebSocket upgrade headers (RFC 6455 §4.2.1).
fn is_websocket_upgrade<B>(req: &HttpRequest<B>) -> bool {
    let has_upgrade_connection = req
        .headers()
        .get(CONNECTION)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.to_ascii_lowercase().contains("upgrade"));

    let has_websocket_upgrade = req
        .headers()
        .get(UPGRADE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"));

    has_upgrade_connection && has_websocket_upgrade
}

impl<S, B> Service<HttpRequest<B>> for WsPassthroughProxy<S>
where
    S: Service<HttpRequest<B>, Response = HttpResponse>
        + Service<HttpRequest, Response = HttpResponse>
        + Clone
        + Send
        + 'static,
    <S as Service<HttpRequest<B>>>::Error: Into<BoxError> + 'static,
    <S as Service<HttpRequest<B>>>::Future: Send + 'static,
    <S as Service<HttpRequest>>::Error: Into<BoxError> + 'static,
    <S as Service<HttpRequest>>::Future: Send + 'static,
    B: http_body::Body<Data = bytes::Bytes> + Send + 'static,
    B::Data: Send,
    B::Error: Into<BoxError>,
{
    type Response = HttpResponse;
    type Error = BoxError;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Service::<HttpRequest<B>>::poll_ready(&mut self.proxy, cx)
    }

    fn call(&mut self, req: HttpRequest<B>) -> Self::Future {
        if is_websocket_upgrade(&req) {
            let fut = Service::<HttpRequest<B>>::call(&mut self.inner, req);
            Box::pin(async move { fut.await.map_err(Into::into) })
        } else {
            Service::<HttpRequest<B>>::call(&mut self.proxy, req)
        }
    }
}
