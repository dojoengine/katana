use anyhow::Result;
use opentelemetry::trace::TracerProvider;
use tracing::Subscriber;
use tracing_subscriber::registry::LookupSpan;

/// Create an OTLP layer exporting tracing data.
pub fn otlp_layer<S>() -> Result<impl tracing_subscriber::Layer<S>>
where
    S: Subscriber + for<'span> LookupSpan<'span>,
{
    let otlp_exporter = opentelemetry_otlp::SpanExporter::builder().with_tonic().build()?;

    // Create a tracer provider with the exporter
    let tracer = opentelemetry_sdk::trace::TracerProviderBuilder::default()
        .with_simple_exporter(otlp_exporter)
        .build();

    Ok(tracing_opentelemetry::layer().with_tracer(tracer.tracer("katana")))
}
