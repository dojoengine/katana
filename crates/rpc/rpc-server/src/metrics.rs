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
use std::sync::Arc;
use std::time::Instant;

use jsonrpsee::core::middleware::{Batch, Notification, RpcServiceT};
use jsonrpsee::types::Request;
use jsonrpsee::{MethodResponse, RpcModule};
use katana_metrics::metrics::{Counter, Histogram};
use katana_metrics::Metrics;
use tower::Layer;

/// Metrics for the RPC server.
#[allow(missing_debug_implementations)]
#[derive(Default, Clone)]
pub struct RpcServerMetrics {
    inner: Arc<RpcServerMetricsInner>,
}

impl RpcServerMetrics {
    /// Creates a new instance of `RpcServerMetrics` for the given `RpcModule`.
    pub fn new(module: &RpcModule<()>) -> Self {
        let call_metrics = HashMap::from_iter(module.method_names().map(|method| {
            let metrics = RpcServerCallMetrics::new_with_labels(&[("method", method)]);
            (method, metrics)
        }));

        Self {
            inner: Arc::new(RpcServerMetricsInner {
                call_metrics,
                connection_metrics: RpcServerConnectionMetrics::default(),
            }),
        }
    }

    /// Creates a new instance of `RpcServerMetrics` with additional labels
    /// on each method's metrics.
    ///
    /// ```rust,ignore
    /// RpcServerMetrics::new_with_labels(module, &[("version", "v0_9")]);
    /// ```
    pub fn new_with_labels(module: &RpcModule<()>, extra_labels: &[(&str, &str)]) -> Self {
        // Leak label strings to get 'static lifetimes, required by the metrics
        // API. This is fine because labels are registered once at startup.
        let extra: Vec<(&'static str, &'static str)> = extra_labels
            .iter()
            .map(|(k, v)| {
                let k: &'static str = Box::leak(k.to_string().into_boxed_str());
                let v: &'static str = Box::leak(v.to_string().into_boxed_str());
                (k, v)
            })
            .collect();

        let call_metrics = HashMap::from_iter(module.method_names().map(|method| {
            let mut labels = vec![("method", method)];
            labels.extend_from_slice(&extra);
            let metrics = RpcServerCallMetrics::new_with_labels(&labels);
            (method, metrics)
        }));

        Self {
            inner: Arc::new(RpcServerMetricsInner {
                call_metrics,
                connection_metrics: RpcServerConnectionMetrics::default(),
            }),
        }
    }
}

#[derive(Default, Clone)]
struct RpcServerMetricsInner {
    /// Connection metrics per transport type
    connection_metrics: RpcServerConnectionMetrics,
    /// Call metrics per RPC method
    call_metrics: HashMap<&'static str, RpcServerCallMetrics>,
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
pub struct RpcServerMetricsLayer {
    metrics: RpcServerMetrics,
}

impl RpcServerMetricsLayer {
    pub fn new(module: &RpcModule<()>) -> Self {
        Self { metrics: RpcServerMetrics::new(module) }
    }

    pub fn new_with_labels(module: &RpcModule<()>, labels: &[(&str, &str)]) -> Self {
        Self { metrics: RpcServerMetrics::new_with_labels(module, labels) }
    }
}

impl<S> Layer<S> for RpcServerMetricsLayer {
    type Service = RpcRequestMetricsService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RpcRequestMetricsService { inner, metrics: self.metrics.clone() }
    }
}

/// Tower service that collects metrics for RPC calls
#[allow(missing_debug_implementations)]
#[derive(Clone)]
pub struct RpcRequestMetricsService<S> {
    inner: S,
    metrics: RpcServerMetrics,
}

impl<S> Drop for RpcRequestMetricsService<S> {
    fn drop(&mut self) {
        self.metrics.inner.connection_metrics.connections_closed.increment(1);
    }
}

impl<S> RpcServiceT for RpcRequestMetricsService<S>
where
    S: RpcServiceT<MethodResponse = MethodResponse> + Send + Sync + Clone + 'static,
{
    type MethodResponse = S::MethodResponse;
    type NotificationResponse = S::NotificationResponse;
    type BatchResponse = S::BatchResponse;

    fn call<'a>(&self, req: Request<'a>) -> impl Future<Output = S::MethodResponse> + Send + 'a {
        let started_at = Instant::now();
        let method = req.method.clone();

        // Record connection metrics
        self.metrics.inner.connection_metrics.connections_opened.increment(1);
        self.metrics.inner.connection_metrics.requests_started.increment(1);

        // Record call metrics
        if let Some(call_metrics) = self.metrics.inner.call_metrics.get(&method.as_ref()) {
            call_metrics.started.increment(1);
        }

        let metrics = self.metrics.clone();
        let fut = self.inner.call(req);

        Box::pin(async move {
            let result = fut.await;

            // Record response metrics
            let time_taken = started_at.elapsed().as_secs_f64();
            metrics.inner.connection_metrics.requests_finished.increment(1);
            metrics.inner.connection_metrics.request_time_seconds.record(time_taken);

            // Record call result metrics
            if let Some(call_metrics) = metrics.inner.call_metrics.get(&method.as_ref()) {
                call_metrics.time_seconds.record(time_taken);

                if result.is_success() {
                    call_metrics.successful.increment(1)
                } else {
                    call_metrics.failed.increment(1)
                }
            }

            result
        })
    }

    fn batch<'a>(&self, req: Batch<'a>) -> impl Future<Output = Self::BatchResponse> + Send + 'a {
        self.inner.batch(req)
    }

    fn notification<'a>(
        &self,
        n: Notification<'a>,
    ) -> impl Future<Output = Self::NotificationResponse> + Send + 'a {
        self.inner.notification(n)
    }
}
