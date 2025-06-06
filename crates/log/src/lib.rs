use opentelemetry::trace::TracerProvider;
use opentelemetry_gcloud_trace::errors::GcloudTraceError;
use opentelemetry_gcloud_trace::{GcpCloudTraceExporterBuilder, SdkTracer};
use opentelemetry_otlp::SpanExporterBuilder;
use opentelemetry_sdk::trace::{RandomIdGenerator, SdkTracerProvider};
use opentelemetry_sdk::Resource;
use tracing::subscriber::SetGlobalDefaultError;
use tracing_log::log::SetLoggerError;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{filter, EnvFilter, Layer};

mod fmt;
pub mod gcloud;
pub mod otel;

pub use fmt::LogFormat;

#[derive(Debug, Clone)]
pub enum TelemetryConfig {
    Otlp(OtlpConfig),
    Gcloud(GcloudConfig),
}

#[derive(Debug, Clone)]
pub struct OtlpConfig {
    pub endpoint: Option<String>,
    pub timeout: u64,
}

#[derive(Debug, Clone)]
pub struct GcloudConfig {
    pub project_id: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to initialize log tracer: {0}")]
    LogTracerInit(#[from] SetLoggerError),

    #[error("failed to parse environment filter: {0}")]
    EnvFilterParse(#[from] filter::ParseError),

    #[error("failed to set global dispatcher: {0}")]
    SetGlobalDefault(#[from] SetGlobalDefaultError),

    #[error("google cloud trace error: {0}")]
    GcloudTrace(#[from] GcloudTraceError),
}
pub async fn init(
    format: LogFormat,
    dev_log: bool,
    telemetry_config: Option<TelemetryConfig>,
) -> Result<(), Error> {
    const DEFAULT_LOG_FILTER: &str = "cairo_native::compiler=off,pipeline=debug,stage=debug,info,\
                                      tasks=debug,executor=trace,forking::backend=trace,\
                                      blockifier=off,jsonrpsee_server=off,hyper=off,\
                                      messaging=debug,node=error,explorer=info";

    let filter = if dev_log {
        format!("{DEFAULT_LOG_FILTER},server=debug")
    } else {
        DEFAULT_LOG_FILTER.to_string()
    };

    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    // If the user has set the `RUST_LOG` environment variable, then we prioritize it.
    // Otherwise, we use the default log filter.
    // TODO: change env var to `KATANA_LOG`.
    let filter = EnvFilter::try_from_default_env().or(EnvFilter::try_new(&filter))?;

    // Initialize tracing subscriber with optional telemetry
    if let Some(telemetry_config) = telemetry_config {
        // Initialize telemetry layer based on exporter type
        let telemetry = match telemetry_config {
            TelemetryConfig::Gcloud(cfg) => {
                let tracer = init_gcp_tracer(&cfg).await?;
                tracing_opentelemetry::layer().with_tracer(tracer)
            }
            TelemetryConfig::Otlp(cfg) => {
                let tracer = init_otlp_tracer(&cfg)?;
                tracing_opentelemetry::layer().with_tracer(tracer)
            }
        };

        let fmt = match format {
            LogFormat::Full => tracing_subscriber::fmt::layer().boxed(),
            LogFormat::Json => tracing_subscriber::fmt::layer().json().boxed(),
        };

        tracing_subscriber::registry().with(filter).with(telemetry).with(fmt).init();
    } else {
        let fmt = match format {
            LogFormat::Full => tracing_subscriber::fmt::layer().boxed(),
            LogFormat::Json => tracing_subscriber::fmt::layer().json().boxed(),
        };

        tracing_subscriber::registry().with(filter).with(fmt).init();
    }

    Ok(())
}

/// Initialize Google Cloud Trace exporter
///
/// Make sure to set `GOOGLE_APPLICATION_CREDENTIALS` env var to authenticate to gcloud
async fn init_gcp_tracer(gcloud_config: &GcloudConfig) -> Result<SdkTracer, Error> {
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
    let tracer = trace_exporter.install(&tracer_provider).await.unwrap();

    opentelemetry::global::set_tracer_provider(tracer_provider.clone());

    Ok(tracer)
}

/// Initialize OTLP tracer
fn init_otlp_tracer(otlp_config: &OtlpConfig) -> Result<opentelemetry_sdk::trace::Tracer, Error> {
    use std::time::Duration;

    use opentelemetry_otlp::WithExportConfig;

    let resource = Resource::builder().with_service_name("katana").build();

    let mut exporter_builder = SpanExporterBuilder::new()
        .with_tonic()
        .with_timeout(Duration::from_secs(otlp_config.timeout));

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
