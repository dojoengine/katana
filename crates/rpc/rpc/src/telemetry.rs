//! Distributed tracing support for RPC requests
//!
//! This module provides middleware for propagating trace context from incoming HTTP requests
//! and adding trace information to structured logs.

use std::task::{Context, Poll};

use anyhow::Result;
use http::{Request, Response};
use opentelemetry::trace::{TraceContextExt, TracerProvider};
use opentelemetry::{global, KeyValue};
use opentelemetry_gcloud_trace::GcpCloudTraceExporterBuilder;
use opentelemetry_http::HeaderExtractor;
use opentelemetry_otlp::{Protocol, WithExportConfig};
use opentelemetry_sdk::Resource;
use tower::{Layer, Service};
use tracing::{error, info, info_span, span, Subscriber};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Registry;

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
    let tracer = opentelemetry_sdk::trace::TracerProviderBuilder::default()
        .with_simple_exporter(otlp_exporter)
        .build()
        .tracer("test");

    Ok(tracing_opentelemetry::layer().with_tracer(tracer))
}
