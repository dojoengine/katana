//! This module is responsible for managing and collecting metrics related to the RPC
//! server. The metrics collected are primarily focused on connections and method calls.
//!
//! ## Connections
//!
//! Metrics related to connections:
//!
//! - Number of connections opened
//! - Number of connections closed
//! - Number of requests started
//! - Number of requests finished
//! - Response time for each request/response pair
//!
//! ## Method Calls
//!
//! Metrics are collected for each methods expose by the RPC server. The metrics collected include:
//!
//! - Number of calls started for each method
//! - Number of successful calls for each method
//! - Number of failed calls for each method
//! - Response time for each method call

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;

use bytes::Bytes;
use jsonrpsee::core::BoxError;
use jsonrpsee::server::ws::is_upgrade_request;
use jsonrpsee::server::{HttpRequest, HttpResponse};
use jsonrpsee::RpcModule;
use katana_metrics::metrics::{Counter, Histogram};
use katana_metrics::Metrics;
use tower::{Layer, Service};

/// Metrics for the RPC server.
#[allow(missing_debug_implementations)]
#[derive(Default, Clone)]
pub struct RpcServerMetrics {
    inner: Arc<RpcServerMetricsInner>,
}

impl RpcServerMetrics {
    /// Creates a new instance of `RpcServerMetrics` for the given `RpcModule`.
    /// This will create metrics for each method in the module.
    pub fn new(module: &RpcModule<()>) -> Self {
        let call_metrics = HashMap::from_iter(module.method_names().map(|method| {
            let metrics = RpcServerCallMetrics::new_with_labels(&[("method", method)]);
            (method, metrics)
        }));

        Self {
            inner: Arc::new(RpcServerMetricsInner {
                call_metrics,
                connection_metrics: ConnectionMetrics::default(),
            }),
        }
    }
}

#[derive(Default, Clone)]
struct RpcServerMetricsInner {
    /// Connection metrics per transport type
    connection_metrics: ConnectionMetrics,
    /// Call metrics per RPC method
    call_metrics: HashMap<&'static str, RpcServerCallMetrics>,
}

#[derive(Clone)]
struct ConnectionMetrics {
    /// Metrics for WebSocket connections
    ws: RpcServerConnectionMetrics,
    /// Metrics for HTTP connections
    http: RpcServerConnectionMetrics,
}

impl ConnectionMetrics {
    /// Returns the metrics for the given transport protocol
    fn get_metrics(&self, is_websocket: bool) -> &RpcServerConnectionMetrics {
        if is_websocket {
            &self.ws
        } else {
            &self.http
        }
    }
}

impl Default for ConnectionMetrics {
    fn default() -> Self {
        Self {
            ws: RpcServerConnectionMetrics::new_with_labels(&[("transport", "ws")]),
            http: RpcServerConnectionMetrics::new_with_labels(&[("transport", "http")]),
        }
    }
}

/// Metrics for the RPC connections
#[derive(Metrics, Clone)]
#[metrics(scope = "rpc_server.connections")]
struct RpcServerConnectionMetrics {
    /// The number of connections opened
    connections_opened: Counter,
    /// The number of connections closed
    connections_closed: Counter,
    /// The number of requests started
    requests_started: Counter,
    /// The number of requests finished
    requests_finished: Counter,
    /// Response for a single request/response pair
    request_time_seconds: Histogram,
}

/// Metrics for the RPC calls
#[derive(Metrics, Clone)]
#[metrics(scope = "rpc_server.calls")]
struct RpcServerCallMetrics {
    /// The number of calls started
    started: Counter,
    /// The number of successful calls
    successful: Counter,
    /// The number of failed calls
    failed: Counter,
    /// Response for a single call
    time_seconds: Histogram,
}

/// Tower layer for RPC server metrics
#[allow(missing_debug_implementations)]
#[derive(Clone)]
pub struct MetricsLayer {
    metrics: RpcServerMetrics,
}

impl MetricsLayer {
    pub fn new(module: &RpcModule<()>) -> Self {
        Self { metrics: RpcServerMetrics::new(module) }
    }
}

impl<S> Layer<S> for MetricsLayer {
    type Service = MetricsService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        MetricsService { service: inner, metrics: self.metrics.clone() }
    }
}

/// Tower service that collects metrics for RPC calls
#[allow(missing_debug_implementations)]
#[derive(Clone)]
pub struct MetricsService<S> {
    service: S,
    metrics: RpcServerMetrics,
}

impl<S, B> Service<HttpRequest<B>> for MetricsService<S>
where
    S: Service<HttpRequest<B>, Response = HttpResponse>,
    S::Error: Into<BoxError> + 'static,
    S::Future: Send + 'static,
    S::Response: 'static,
    B::Data: Send,
    B::Error: Into<BoxError>,
    B: http_body::Body<Data = Bytes> + Send + 'static,
{
    type Error = S::Error;
    type Response = S::Response;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&mut self, req: HttpRequest<B>) -> Self::Future {
        let is_ws = is_upgrade_request(&req);

        let started_at = Instant::now();
        let method = req.method().to_string();

        // Record connection metrics
        let connection_metrics = self.metrics.inner.connection_metrics.get_metrics(is_ws);
        connection_metrics.requests_started.increment(1);

        // Record call metrics
        if let Some(call_metrics) = self.metrics.inner.call_metrics.get(method.as_str()) {
            call_metrics.started.increment(1);
        }

        let metrics = self.metrics.clone();
        let fut = self.service.call(req);

        Box::pin(async move {
            let result = fut.await;

            // Record response metrics
            let time_taken = started_at.elapsed().as_secs_f64();
            let connection_metrics = metrics.inner.connection_metrics.get_metrics(is_ws);
            connection_metrics.request_time_seconds.record(time_taken);
            connection_metrics.requests_finished.increment(1);

            // Record call result metrics
            if let Some(call_metrics) = metrics.inner.call_metrics.get(method.as_str()) {
                call_metrics.time_seconds.record(time_taken);

                match &result {
                    Ok(_) => call_metrics.successful.increment(1),
                    Err(_) => call_metrics.failed.increment(1),
                }
            }

            result
        })
    }
}
