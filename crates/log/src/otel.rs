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

/// Initialize OTLP tracer
pub fn init_otlp_tracer(
    otlp_config: &OtlpConfig,
) -> Result<opentelemetry_sdk::trace::Tracer, Error> {
    use opentelemetry_otlp::WithExportConfig;

    let resource = Resource::builder().with_service_name("katana").build();

    let mut exporter_builder = SpanExporterBuilder::new().with_tonic();

    if let Some(endpoint) = &otlp_config.endpoint {
        exporter_builder = exporter_builder.with_endpoint(endpoint);
    }

    let exporter = exporter_builder.build().unwrap();

    let provider = SdkTracerProvider::builder()
        .with_id_generator(RandomIdGenerator::default())
        .with_batch_exporter(exporter)
        .with_resource(resource)
        .build();

    opentelemetry::global::set_tracer_provider(provider.clone());

    Ok(provider.tracer("katana"))
}
