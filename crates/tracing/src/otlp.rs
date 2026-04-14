use anyhow::Result;
use opentelemetry::trace::TracerProvider;
use opentelemetry_otlp::SpanExporterBuilder;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::{RandomIdGenerator, SdkTracerProvider};
use opentelemetry_sdk::Resource;

use crate::Error;

#[derive(Debug, Clone)]
pub struct OtlpConfig {
    pub endpoint: Option<String>,
}

/// Initialize OTLP tracer
pub fn init_tracer(otlp_config: &OtlpConfig) -> Result<opentelemetry_sdk::trace::Tracer, Error> {
    use opentelemetry_otlp::WithExportConfig;

    let resource = Resource::builder().with_service_name("katana").build();

    let mut exporter_builder = SpanExporterBuilder::new().with_tonic();

    if let Some(endpoint) = &otlp_config.endpoint {
        exporter_builder = exporter_builder.with_endpoint(endpoint);
    }

    let exporter = exporter_builder.build()?;

    let provider = SdkTracerProvider::builder()
        .with_id_generator(RandomIdGenerator::default())
        .with_batch_exporter(exporter)
        .with_resource(resource)
        .build();

    // Install a W3C TraceContext propagator so the `MakeSpan` used by the
    // RPC server's `tower_http::TraceLayer` can extract inbound `traceparent`
    // headers and chain exported spans under the caller's trace_id. Without
    // this, every inbound request starts a fresh root trace even when the
    // caller sent a trace context — breaking distributed tracing across
    // services. The `gcloud` path installs its own propagator; the OTLP
    // path was missing one.
    opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());

    opentelemetry::global::set_tracer_provider(provider.clone());

    Ok(provider.tracer("katana"))
}
