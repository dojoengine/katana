//! Distributed tracing support for RPC requests
//!
//! This module provides middleware for propagating trace context from incoming HTTP requests
//! and adding trace information to structured logs.

use std::task::{Context, Poll};

use anyhow::Result;
use http::{Request, Response};
use opentelemetry::global;
use opentelemetry::trace::TraceContextExt;
use opentelemetry::trace::TracerProvider;
use opentelemetry_http::HeaderExtractor;
use opentelemetry_otlp::{Protocol, WithExportConfig};
use tower::{Layer, Service};
use tracing::{info_span, Subscriber};
use tracing_subscriber::registry::LookupSpan;

/// Layer that adds trace context propagation to HTTP requests
#[derive(Debug, Clone)]
pub struct TraceContextLayer;

impl TraceContextLayer {
    pub fn new() -> Self {
        Self
    }
}

impl<S> Layer<S> for TraceContextLayer {
    type Service = TraceContextService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        TraceContextService { inner }
    }
}

/// Service that extracts trace context from HTTP headers and adds trace information to logs
#[derive(Debug, Clone)]
pub struct TraceContextService<S> {
    inner: S,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for TraceContextService<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request<ReqBody>) -> Self::Future {
        // Extract trace context from HTTP headers
        let parent_context = global::get_text_map_propagator(|propagator| {
            propagator.extract(&HeaderExtractor(request.headers()))
        });

        let span_context = parent_context.span().span_context();

        if !span_context.is_valid() {
            return self.inner.call(request);
        }

        let trace_id = span_context.trace_id();
        let span_id = span_context.span_id();

        // Format for Google Cloud structured logging
        let trace_id_hex = format!("{:032x}", trace_id);
        let span_id_hex = format!("{:016x}", span_id);

        // Check if we have a project ID from environment for full trace path
        let trace_field = if let Ok(project_id) = std::env::var("GOOGLE_CLOUD_PROJECT") {
            format!("projects/{}/traces/{}", project_id, trace_id_hex)
        } else {
            // Fallback to just the trace ID if no project ID is available
            trace_id_hex
        };

        let span = info_span!(
            "rpc_request",
            trace = %trace_field,
            spanId = %span_id_hex,
            // Keep the original fields for backward compatibility
            trace_id = %trace_id,
            span_id = %span_id
        );

        let _enter = span.enter();
        self.inner.call(request)
    }
}

/// Initialize OpenTelemetry propagators for Google Cloud trace context support
///
/// This function should be called during application startup to configure
/// the global text map propagator to support Google Cloud's X-Cloud-Trace-Context headers.
pub fn init_trace_propagation() {
    use opentelemetry_stackdriver::google_trace_context_propagator::GoogleTraceContextPropagator;

    // Set the Google Cloud trace context propagator globally
    // This will handle both extraction and injection of X-Cloud-Trace-Context headers
    global::set_text_map_propagator(GoogleTraceContextPropagator::default());
}

/// Create an OTLP layer exporting tracing data.
fn otlp_layer<S>() -> Result<impl tracing_subscriber::Layer<S>>
where
    S: Subscriber + for<'span> LookupSpan<'span>,
{
    // Initialize OTLP exporter using HTTP binary protocol
    let otlp_exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_protocol(Protocol::HttpBinary)
        .build()?;

    // Create a tracer provider with the exporter
    let tracer = opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_simple_exporter(otlp_exporter)
        .build()
        .tracer("test");

    Ok(tracing_opentelemetry::layer().with_tracer(tracer))
}
