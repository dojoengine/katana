use opentelemetry::trace::{Tracer, TracerProvider};
use opentelemetry_otlp::{SpanExporterBuilder, WithExportConfig};
use opentelemetry_sdk::trace::{RandomIdGenerator, SdkTracerProvider};
use opentelemetry_sdk::Resource;
use tracing_opentelemetry::PreSampledTracer;

use crate::{Error, TelemetryTracer};

#[derive(Debug, Clone)]
pub struct OtlpConfig {
    pub endpoint: Option<String>,
}

/// Builder for creating an OpenTelemetry layer with OTLP exporter
#[derive(Debug, Clone)]
pub struct OtlpTracerBuilder {
    service_name: String,
    endpoint: Option<String>,
    resource: Option<Resource>,
}

impl OtlpTracerBuilder {
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
    pub fn build(self) -> Result<OtlpTracer, Error> {
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
        let tracer_provider = SdkTracerProvider::builder()
            .with_id_generator(RandomIdGenerator::default())
            .with_batch_exporter(exporter)
            .with_resource(resource)
            .build();

        // // Set global provider
        // opentelemetry::global::set_tracer_provider(tracer_provider.clone());
        let tracer = tracer_provider.tracer(self.service_name);

        Ok(OtlpTracer { tracer, tracer_provider })
    }
}

impl Default for OtlpTracerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Wrapper type for SdkTracer that implements the Tracer trait
#[derive(Debug, Clone)]
pub struct OtlpTracer {
    tracer: opentelemetry_sdk::trace::Tracer,
    tracer_provider: SdkTracerProvider,
}

impl OtlpTracer {
    pub fn builder() -> OtlpTracerBuilder {
        OtlpTracerBuilder::new()
    }
}

impl Tracer for OtlpTracer {
    type Span = <opentelemetry_sdk::trace::Tracer as Tracer>::Span;

    #[inline]
    fn build_with_context(
        &self,
        builder: opentelemetry::trace::SpanBuilder,
        parent_cx: &opentelemetry::Context,
    ) -> Self::Span {
        self.tracer.build_with_context(builder, parent_cx)
    }
}

impl PreSampledTracer for OtlpTracer {
    #[inline]
    fn new_span_id(&self) -> opentelemetry::trace::SpanId {
        self.tracer.new_span_id()
    }

    #[inline]
    fn new_trace_id(&self) -> opentelemetry::trace::TraceId {
        self.tracer.new_trace_id()
    }

    #[inline]
    fn sampled_context(
        &self,
        data: &mut tracing_opentelemetry::OtelData,
    ) -> opentelemetry::Context {
        self.tracer.sampled_context(data)
    }
}

impl TelemetryTracer for OtlpTracer {
    fn init(&self) -> Result<(), Error> {
        // Set global provider
        opentelemetry::global::set_tracer_provider(self.tracer_provider.clone());
        Ok(())
    }
}

impl crate::__private::Sealed for OtlpTracer {}
