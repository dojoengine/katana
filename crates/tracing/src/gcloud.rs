use http::Request;
use opentelemetry::trace::Tracer;
use opentelemetry_gcloud_trace::{GcpCloudTraceExporterBuilder, SdkTracer};
use opentelemetry_http::HeaderExtractor;
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_sdk::Resource;
use opentelemetry_stackdriver::google_trace_context_propagator::GoogleTraceContextPropagator;
use tower_http::trace::MakeSpan;
use tracing_opentelemetry::{OpenTelemetrySpanExt, PreSampledTracer};

use crate::{Error, TelemetryTracer};

#[derive(Debug, Clone, Default)]
pub struct GoogleStackDriverMakeSpan;

impl<B> MakeSpan<B> for GoogleStackDriverMakeSpan {
    fn make_span(&mut self, request: &Request<B>) -> tracing::Span {
        // Extract trace context from HTTP headers
        let cx = opentelemetry::global::get_text_map_propagator(|propagator| {
            propagator.extract(&HeaderExtractor(request.headers()))
        });

        // Create a span from the parent context
        let span = tracing::info_span!(
            "http_request",
            method = %request.method(),
            uri = %request.uri(),
        );
        span.set_parent(cx);

        span
    }
}

#[derive(Debug, Clone)]
pub struct GcloudConfig {
    pub project_id: Option<String>,
}

/// Builder for creating an OpenTelemetry layer with Google Cloud Trace exporter
#[derive(Debug, Clone)]
pub struct GCloudTracerBuilder {
    service_name: String,
    project_id: Option<String>,
    resource: Option<Resource>,
}

/////////////////////////////////////////////////////////////////////////////////
// GCloudTracerBuilder implementations
/////////////////////////////////////////////////////////////////////////////////

impl GCloudTracerBuilder {
    /// Create a new Google Cloud tracing builder
    pub fn new() -> Self {
        Self { service_name: "katana".to_string(), project_id: None, resource: None }
    }

    /// Set the service name
    pub fn service_name(mut self, name: impl Into<String>) -> Self {
        self.service_name = name.into();
        self
    }

    /// Set the Google Cloud project ID
    pub fn project_id(mut self, project_id: impl Into<String>) -> Self {
        self.project_id = Some(project_id.into());
        self
    }

    /// Set a custom resource
    pub fn resource(mut self, resource: Resource) -> Self {
        self.resource = Some(resource);
        self
    }

    /// Build the OpenTelemetry layer (async because GCloud SDK requires it)
    pub async fn build(self) -> Result<GCloudTracer, Error> {
        // Install crypto provider
        rustls::crypto::ring::default_provider()
            .install_default()
            .map_err(|_| Error::InstallCryptoFailed)?;

        // Build resource with service name
        let resource = self.resource.unwrap_or_else(|| {
            Resource::builder().with_service_name(self.service_name.clone()).build()
        });

        // Create trace exporter
        let mut trace_exporter = if let Some(project_id) = self.project_id {
            GcpCloudTraceExporterBuilder::new(project_id)
        } else {
            // Default will attempt to find project ID from environment variables in the following
            // order:
            // - GCP_PROJECT
            // - PROJECT_ID
            // - GCP_PROJECT_ID
            GcpCloudTraceExporterBuilder::for_default_project_id().await?
        };

        trace_exporter = trace_exporter.with_resource(resource);

        // Create provider and install
        let tracer_provider = trace_exporter.create_provider().await?;
        let tracer = trace_exporter.install(&tracer_provider).await?;

        // // Set the Google Cloud trace context propagator globally
        // // This will handle both extraction and injection of X-Cloud-Trace-Context headers
        // opentelemetry::global::set_text_map_propagator(GoogleTraceContextPropagator::default());
        // opentelemetry::global::set_tracer_provider(tracer_provider.clone());

        // Return the layer
        Ok(GCloudTracer { tracer, tracer_provider })
    }
}

impl Default for GCloudTracerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Wrapper type for SdkTracer that implements the Tracer trait
#[derive(Debug, Clone)]
pub struct GCloudTracer {
    tracer: SdkTracer,
    tracer_provider: SdkTracerProvider,
}

/////////////////////////////////////////////////////////////////////////////////
// GCloudTracer implementations
/////////////////////////////////////////////////////////////////////////////////

impl GCloudTracer {
    pub fn builder() -> GCloudTracerBuilder {
        GCloudTracerBuilder::new()
    }
}

impl Tracer for GCloudTracer {
    type Span = <SdkTracer as Tracer>::Span;

    #[inline]
    fn build_with_context(
        &self,
        builder: opentelemetry::trace::SpanBuilder,
        parent_cx: &opentelemetry::Context,
    ) -> Self::Span {
        self.tracer.build_with_context(builder, parent_cx)
    }
}

impl PreSampledTracer for GCloudTracer {
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

impl TelemetryTracer for GCloudTracer {
    fn init(&self) -> Result<(), Error> {
        // Set the Google Cloud trace context propagator globally
        // This will handle both extraction and injection of X-Cloud-Trace-Context headers
        opentelemetry::global::set_text_map_propagator(GoogleTraceContextPropagator::default());
        opentelemetry::global::set_tracer_provider(self.tracer_provider.clone());
        Ok(())
    }
}

impl crate::__private::Sealed for GCloudTracer {}
