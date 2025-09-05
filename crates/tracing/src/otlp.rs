use anyhow::Result;
use opentelemetry::trace::TracerProvider;
use opentelemetry_otlp::SpanExporterBuilder;
use opentelemetry_sdk::trace::{RandomIdGenerator, SdkTracerProvider};
use opentelemetry_sdk::Resource;

use crate::Error;

#[derive(Debug, Clone)]
pub struct OtlpConfig {
    pub endpoint: Option<String>,
}

/// Initialize OTLP tracer with custom service name
pub fn init_tracer_with_service(
    otlp_config: &OtlpConfig,
    service_name: &str,
) -> Result<opentelemetry_sdk::trace::Tracer, Error> {
    use opentelemetry_otlp::WithExportConfig;

    let resource = Resource::builder().with_service_name(service_name.to_string()).build();

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

    opentelemetry::global::set_tracer_provider(provider.clone());

    Ok(provider.tracer(service_name.to_string()))
}

/// Initialize OTLP tracer (backward compatibility)
pub fn init_tracer(otlp_config: &OtlpConfig) -> Result<opentelemetry_sdk::trace::Tracer, Error> {
    init_tracer_with_service(otlp_config, "katana")
}
