use opentelemetry::trace::TracerProvider;
use opentelemetry_otlp::{SpanExporterBuilder, WithExportConfig};
use opentelemetry_sdk::trace::{RandomIdGenerator, SdkTracerProvider, Tracer};
use opentelemetry_sdk::Resource;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::Registry;

use crate::Error;

#[derive(Debug, Clone)]
pub struct OtlpConfig {
    pub endpoint: Option<String>,
}

/// Builder for creating an OpenTelemetry layer with OTLP exporter
#[derive(Debug, Clone)]
pub struct OtlpTracingBuilder {
    service_name: String,
    endpoint: Option<String>,
    resource: Option<Resource>,
}

impl OtlpTracingBuilder {
    /// Create a new OTLP tracing builder
    pub fn new() -> Self {
        Self { service_name: "katana".to_string(), endpoint: None, resource: None }
    }

    /// Set the service name
    pub fn service_name(mut self, name: impl Into<String>) -> Self {
        self.service_name = name.into();
        self
    }

    /// Set the OTLP endpoint
    pub fn endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }

    /// Set a custom resource
    pub fn resource(mut self, resource: Resource) -> Self {
        self.resource = Some(resource);
        self
    }

    /// Build the OpenTelemetry layer
    pub fn build(self) -> Result<OpenTelemetryLayer<Registry, Tracer>, Error> {
        // Build resource with service name
        let resource = self.resource.unwrap_or_else(|| {
            Resource::builder().with_service_name(self.service_name.clone()).build()
        });

        // Configure exporter
        let mut exporter_builder = SpanExporterBuilder::new().with_tonic();

        if let Some(endpoint) = self.endpoint {
            exporter_builder = exporter_builder.with_endpoint(endpoint);
        }

        let exporter = exporter_builder.build()?;

        // Build provider
        let provider = SdkTracerProvider::builder()
            .with_id_generator(RandomIdGenerator::default())
            .with_batch_exporter(exporter)
            .with_resource(resource)
            .build();

        // Set global provider
        opentelemetry::global::set_tracer_provider(provider.clone());

        // Create tracer
        let tracer = provider.tracer(self.service_name);

        // Return the layer
        Ok(tracing_opentelemetry::layer().with_tracer(tracer))
    }
}

impl Default for OtlpTracingBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Initialize OTLP tracer with custom service name (backward compatibility)
pub fn init_tracer_with_service(
    otlp_config: &OtlpConfig,
    service_name: &str,
) -> Result<Tracer, Error> {
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
pub fn init_tracer(otlp_config: &OtlpConfig) -> Result<Tracer, Error> {
    init_tracer_with_service(otlp_config, "katana")
}
