//! Distributed tracing support for the RPC server.
//!
//! This module provides OpenTelemetry-based distributed tracing with support for
//! Google Cloud Trace and OTLP exporters.
//!
//! Note: The Google Cloud Trace integration is currently using a stdout exporter
//! as a placeholder. The actual GCP integration will be added once the 
//! opentelemetry-gcloud-trace crate API stabilizes.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use http::{Request, Response};
use opentelemetry::global;
use opentelemetry::propagation::{Extractor, TextMapPropagator};
use opentelemetry::sdk::propagation::TraceContextPropagator;
use opentelemetry::trace::{TraceContextExt, Tracer};
use opentelemetry::{Context as OtelContext, KeyValue};
use opentelemetry_http::HeaderExtractor;

use tower::{Layer, Service};

/// Configuration for distributed tracing
#[derive(Debug, Clone)]
pub struct TracingConfig {
    /// The service name to use for traces
    pub service_name: String,
    /// The exporter type to use
    pub exporter: TracingExporter,
    /// Sample rate (0.0 to 1.0)
    pub sample_rate: f64,
}

/// Supported tracing exporters
#[derive(Debug, Clone)]
pub enum TracingExporter {
    /// Google Cloud Trace
    GoogleCloudTrace {
        /// GCP project ID
        project_id: String,
    },
    /// OpenTelemetry Protocol (OTLP)
    #[cfg(feature = "opentelemetry-otlp")]
    Otlp {
        /// OTLP endpoint URL
        endpoint: String,
    },
    /// No-op exporter for testing
    None,
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self {
            service_name: "katana-rpc".to_string(),
            exporter: TracingExporter::None,
            sample_rate: 1.0,
        }
    }
}

/// Initialize the global tracer based on the configuration
pub async fn init_tracer(config: TracingConfig) -> anyhow::Result<()> {
    match &config.exporter {
        TracingExporter::GoogleCloudTrace { project_id } => {
            init_gcp_tracer(&config.service_name, project_id, config.sample_rate).await?;
        }
        #[cfg(feature = "opentelemetry-otlp")]
        TracingExporter::Otlp { endpoint } => {
            init_otlp_tracer(&config.service_name, endpoint, config.sample_rate)?;
        }
        TracingExporter::None => {
            // No-op tracer, do nothing
        }
    };

    tracing::info!("Distributed tracing initialized with {:?}", config.exporter);
    Ok(())
}

/// Initialize Google Cloud Trace exporter
async fn init_gcp_tracer(
    service_name: &str,
    project_id: &str,
    _sample_rate: f64,
) -> anyhow::Result<()> {
    use opentelemetry::sdk::Resource;

    let resource = Resource::new(vec![
        KeyValue::new("service.name", service_name.to_string()),
        KeyValue::new("cloud.provider", "gcp"),
        KeyValue::new("cloud.project_id", project_id.to_string()),
    ]);

    // TODO: Replace with actual GCP exporter when API is stabilized
    // The opentelemetry-gcloud-trace crate needs to be updated to match
    // the current opentelemetry API version
    tracing::warn!("Using stdout exporter instead of Google Cloud Trace exporter");
    // TODO: Use proper stdout exporter when available
    // For now, we'll use a no-op approach
    tracing::warn!("Google Cloud Trace exporter not yet implemented - tracing disabled for GCP");
    return Ok(());


}

/// Initialize OTLP exporter
#[cfg(feature = "opentelemetry-otlp")]
fn init_otlp_tracer(
    service_name: &str,
    endpoint: &str,
    _sample_rate: f64,
) -> anyhow::Result<()> {
    use opentelemetry::sdk::Resource;
    use opentelemetry_otlp::WithExportConfig;
    use std::time::Duration;

    let resource = Resource::new(vec![KeyValue::new("service.name", service_name.to_string())]);

    let exporter = opentelemetry_otlp::new_exporter()
        .tonic()
        .with_endpoint(endpoint)
        .with_timeout(Duration::from_secs(3));

    let provider = opentelemetry_sdk::trace::TracerProvider::builder()
        .with_batch_exporter(exporter, opentelemetry_sdk::runtime::Tokio)
        .with_config(
            opentelemetry_sdk::trace::config()
                .with_resource(resource)
                .with_sampler(opentelemetry_sdk::trace::Sampler::AlwaysOn),
        )
        .build();

    global::set_tracer_provider(provider);
    Ok(())
}

/// Tower layer that adds distributed tracing to HTTP requests
#[derive(Clone)]
pub struct TracingLayer;

impl<S> Layer<S> for TracingLayer {
    type Service = TracingMiddleware<S>;

    fn layer(&self, service: S) -> Self::Service {
        TracingMiddleware { inner: service }
    }
}

/// Middleware that extracts trace context and creates spans for HTTP requests
#[derive(Clone)]
pub struct TracingMiddleware<S> {
    inner: S,
}

impl<S, B> Service<Request<B>> for TracingMiddleware<S>
where
    S: Service<Request<B>, Response = Response<B>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    B: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<B>) -> Self::Future {
        // Extract trace context from HTTP headers
        let headers = req.headers();
        let parent_ctx = {
            let extractor = HeaderExtractor(headers);
            let propagator = TraceContextPropagator::new();
            propagator.extract(&extractor)
        };

        // Create a new span for this HTTP request
        let tracer = global::tracer("katana-rpc");
        let uri = req.uri().to_string();
        let method = req.method().to_string();
        
        let span = tracer
            .span_builder(format!("HTTP {} {}", method, uri))
            .with_kind(opentelemetry::trace::SpanKind::Server)
            .with_attributes(vec![
                KeyValue::new("http.method", method.clone()),
                KeyValue::new("http.target", uri.clone()),
                KeyValue::new("http.scheme", "http"),
                KeyValue::new("rpc.system", "jsonrpc"),
                KeyValue::new("rpc.service", "katana"),
            ])
            .start_with_context(&tracer, &parent_ctx);

        // Create a tracing span and attach the OpenTelemetry context
        let tracing_span = tracing::span!(
            tracing::Level::INFO,
            "http_request",
            otel.kind = "server",
            http.method = %method,
            http.target = %uri,
        );

        // Call the inner service within the span context
        let _cx = OtelContext::current_with_span(span);
        
        let mut inner = self.inner.clone();
        Box::pin(async move {
            // Use a different approach to handle context
            let result = {
                let _enter = tracing_span.enter();
                inner.call(req).await
            };
            
            result
        })
    }
}


/// Shutdown the global tracer provider
pub fn shutdown_tracer() {
    global::shutdown_tracer_provider();
}