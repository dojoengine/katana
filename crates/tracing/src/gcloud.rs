use http::Request;
use opentelemetry_gcloud_trace::{GcpCloudTraceExporterBuilder, SdkTracer};
use opentelemetry_http::HeaderExtractor;
use opentelemetry_sdk::Resource;
use opentelemetry_stackdriver::google_trace_context_propagator::GoogleTraceContextPropagator;
use tower_http::trace::MakeSpan;
use tracing::Span;
use tracing_opentelemetry::OpenTelemetrySpanExt;

use crate::Error;

#[derive(Debug, Clone, Default)]
pub struct GoogleStackDriverMakeSpan;

impl<B> MakeSpan<B> for GoogleStackDriverMakeSpan {
    fn make_span(&mut self, request: &Request<B>) -> Span {
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

/// Initialize Google Cloud Trace exporter and OpenTelemetry propagators for Google Cloud trace
/// context support.
///
/// Make sure to set `GOOGLE_APPLICATION_CREDENTIALS` env var to authenticate to gcloud
pub(crate) async fn init_tracer(gcloud_config: &GcloudConfig) -> Result<SdkTracer, Error> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| Error::InstallCryptoFailed)?;

    let resource = Resource::builder().with_service_name("katana").build();

    let mut trace_exporter = if let Some(project_id) = &gcloud_config.project_id {
        GcpCloudTraceExporterBuilder::new(project_id.clone())
    } else {
        // Default will attempt to find project ID from environment variables in the following
        // order:
        // - GCP_PROJECT
        // - PROJECT_ID
        // - GCP_PROJECT_ID
        GcpCloudTraceExporterBuilder::for_default_project_id().await?
    };

    trace_exporter = trace_exporter.with_resource(resource);

    let tracer_provider = trace_exporter.create_provider().await?;
    let tracer = trace_exporter.install(&tracer_provider).await?;

    // Set the Google Cloud trace context propagator globally
    // This will handle both extraction and injection of X-Cloud-Trace-Context headers
    opentelemetry::global::set_text_map_propagator(GoogleTraceContextPropagator::default());
    opentelemetry::global::set_tracer_provider(tracer_provider.clone());

    Ok(tracer)
}
