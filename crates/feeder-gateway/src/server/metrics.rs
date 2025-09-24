//! This module is responsible for managing and collecting metrics related to the gateway
//! server. The metrics collected are primarily focused on HTTP connections and endpoint calls.
//!
//! ## Connections
//!
//! Metrics related to HTTP connections:
//!
//! - Number of requests started
//! - Number of requests finished
//! - Response time for each request/response pair
//!
//! ## Endpoint Calls
//!
//! Metrics are collected for each endpoint exposed by the gateway server. The metrics collected
//! include:
//!
//! - Number of calls started for each endpoint
//! - Number of successful calls for each endpoint
//! - Number of failed calls for each endpoint
//! - Response time for each endpoint call

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;

use http::{Request, Response};
use http_body::Body;
use katana_metrics::metrics::{Counter, Histogram};
use katana_metrics::Metrics;
use tower::{Layer, Service};

/// Tower layer for gateway server metrics
#[derive(Clone)]
pub struct GatewayMetricsLayer {
    metrics: GatewayMetrics,
}

impl GatewayMetricsLayer {
    pub fn new<I: IntoIterator<Item = &'static str>>(endpoints: I) -> Self {
        Self { metrics: GatewayMetrics::new(endpoints) }
    }
}

impl fmt::Debug for GatewayMetricsLayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GatewayServerMetricsLayer").finish()
    }
}

impl<S> Layer<S> for GatewayMetricsLayer {
    type Service = GatewayRequestMetricsService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        GatewayRequestMetricsService { inner, metrics: self.metrics.clone() }
    }
}

/// Tower service that collects metrics for gateway requests
#[derive(Clone)]
pub struct GatewayRequestMetricsService<S> {
    inner: S,
    metrics: GatewayMetrics,
}

impl<S> fmt::Debug for GatewayRequestMetricsService<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GatewayRequestMetricsService").finish()
    }
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for GatewayRequestMetricsService<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>>,
    <S as Service<Request<ReqBody>>>::Future: Send + 'static,
    ReqBody: Body,
    ResBody: Body,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        let endpoint = req.uri().path().to_string();

        // Record connection metrics
        self.metrics.inner.connection_metrics.requests_started.increment(1);

        // Record endpoint metrics
        if let Some(endpoint_metrics) = self.metrics.inner.endpoint_metrics.get(endpoint.as_str()) {
            endpoint_metrics.started.increment(1);
        }

        let metrics = self.metrics.clone();
        let fut = self.inner.call(req);

        Box::pin(async move {
            let started_at = Instant::now();
            let result = fut.await;

            // Record response metrics
            let time_taken = started_at.elapsed().as_secs_f64();
            metrics.inner.connection_metrics.requests_finished.increment(1);
            metrics.inner.connection_metrics.request_time_seconds.record(time_taken);

            // Record endpoint result metrics
            if let Some(endpoint_metrics) = metrics.inner.endpoint_metrics.get(endpoint.as_str()) {
                endpoint_metrics.time_seconds.record(time_taken);

                match &result {
                    Ok(response) => {
                        if response.status().is_success() {
                            endpoint_metrics.successful.increment(1);
                        } else {
                            endpoint_metrics.failed.increment(1);
                        }
                    }
                    Err(_) => {
                        endpoint_metrics.failed.increment(1);
                    }
                }
            }

            result
        })
    }
}

/// Metrics for the gateway server.
#[allow(missing_debug_implementations)]
#[derive(Clone)]
struct GatewayMetrics {
    inner: Arc<GatewayMetricsInner>,
}

impl GatewayMetrics {
    fn new<I: IntoIterator<Item = &'static str>>(endpoints: I) -> Self {
        let endpoint_metrics = HashMap::from_iter(endpoints.into_iter().map(|endpoint| {
            let metrics = GatewayServerEndpointMetrics::new_with_labels(&[("endpoint", endpoint)]);
            (endpoint, metrics)
        }));

        Self {
            inner: Arc::new(GatewayMetricsInner {
                endpoint_metrics,
                connection_metrics: GatewayServerConnectionMetrics::default(),
            }),
        }
    }
}

#[derive(Default, Clone)]
struct GatewayMetricsInner {
    /// Connection metrics for HTTP requests
    connection_metrics: GatewayServerConnectionMetrics,
    /// Endpoint metrics per gateway endpoint
    endpoint_metrics: HashMap<&'static str, GatewayServerEndpointMetrics>,
}

/// Metrics for the HTTP connections
#[derive(Metrics, Clone)]
#[metrics(scope = "gateway_server.connections")]
struct GatewayServerConnectionMetrics {
    /// The number of requests started
    requests_started: Counter,
    /// The number of requests finished
    requests_finished: Counter,
    /// Response time for a single request/response pair
    request_time_seconds: Histogram,
}

/// Metrics for the gateway endpoint calls
#[derive(Metrics, Clone)]
#[metrics(scope = "gateway_server.endpoints")]
struct GatewayServerEndpointMetrics {
    /// The number of calls started
    started: Counter,
    /// The number of successful calls
    successful: Counter,
    /// The number of failed calls
    failed: Counter,
    /// Response time for a single call
    time_seconds: Histogram,
}
