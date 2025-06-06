use http::Request;
use opentelemetry_http::HeaderExtractor;
use tower_http::trace::MakeSpan;
use tracing::Span;
use tracing_opentelemetry::OpenTelemetrySpanExt;

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

/// Initialize OpenTelemetry propagators for Google Cloud trace context support
///
/// This function should be called during application startup to configure
/// the global text map propagator to support Google Cloud's X-Cloud-Trace-Context headers.
pub fn init_trace_propagator() {
    use opentelemetry_stackdriver::google_trace_context_propagator::GoogleTraceContextPropagator;
    // Set the Google Cloud trace context propagator globally
    // This will handle both extraction and injection of X-Cloud-Trace-Context headers
    opentelemetry::global::set_text_map_propagator(GoogleTraceContextPropagator::default());
}
