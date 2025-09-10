use opentelemetry::trace::Tracer;
use opentelemetry_gcloud_trace::errors::GcloudTraceError;
use tracing::subscriber::SetGlobalDefaultError;
use tracing_log::log::SetLoggerError;
use tracing_opentelemetry::PreSampledTracer;
use tracing_subscriber::filter;

mod builder;
mod fmt;
pub mod gcloud;
pub mod otlp;

pub use builder::TracingBuilder;
pub use fmt::LogFormat;
pub use gcloud::{GCloudTracerBuilder, GcloudConfig};
pub use otlp::{OtlpConfig, OtlpTracerBuilder};

use crate::builder::NoopTracer;

trait TelemetryTracer: Tracer + PreSampledTracer + Send + Sync + 'static {
    fn init(&self) -> Result<(), Error>;
}

impl TelemetryTracer for NoopTracer {
    fn init(&self) -> Result<(), Error> {
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum TracerConfig {
    Otlp(otlp::OtlpConfig),
    GCloud(gcloud::GcloudConfig),
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to initialize log tracer: {0}")]
    LogTracerInit(#[from] SetLoggerError),

    #[error("failed to parse environment filter: {0}")]
    EnvFilterParse(#[from] filter::ParseError),

    #[error("failed to parse environment filter from env: {0}")]
    EnvFilterFromEnv(#[from] filter::FromEnvError),

    #[error("failed to set global dispatcher: {0}")]
    SetGlobalDefault(#[from] SetGlobalDefaultError),

    #[error("google cloud trace error: {0}")]
    GcloudTrace(#[from] GcloudTraceError),

    #[error("failed to install crypto provider")]
    InstallCryptoFailed,

    #[error("failed to build otlp tracer: {0}")]
    OtlpBuild(#[from] opentelemetry_otlp::ExporterBuildError),

    #[error(transparent)]
    OtelSdk(#[from] opentelemetry_sdk::error::OTelSdkError),
}

/// Initialize tracing with the given configuration.
///
/// This function is maintained for backward compatibility.
/// For new code, consider using [`TracingBuilder`] with the new telemetry builders.
///
/// # Example
/// ```rust,ignore
/// use katana_tracing::{OtlpTracingBuilder, TracingBuilder};
///
/// // New API (recommended):
/// let otlp_layer = OtlpTracingBuilder::new()
///     .service_name("my-service")
///     .endpoint("http://localhost:4317")
///     .build()?;
///
/// TracingBuilder::new()
///     .json()
///     .with_default_filter()?
///     .init_with_otlp_telemetry(otlp_layer)?;
/// ```
pub async fn init(format: LogFormat, telemetry_config: Option<TracerConfig>) -> Result<(), Error> {
    // Build the base tracing builder with format and filter
    let builder = TracingBuilder::with_format(format).with_env_filter_or_default()?;

    // Build telemetry layer and initialize based on config type
    match telemetry_config {
        Some(TracerConfig::Otlp(cfg)) => {
            // OTLP is synchronous
            let mut otlp_builder = OtlpTracerBuilder::new().service_name("katana");
            if let Some(endpoint) = cfg.endpoint {
                otlp_builder = otlp_builder.endpoint(endpoint);
            }
            let layer = otlp_builder.build()?;
            builder.init_with_otlp_telemetry(layer)?;
        }
        Some(TracerConfig::GCloud(cfg)) => {
            // GCloud is async
            let mut gcloud_builder = GCloudTracerBuilder::new().service_name("katana");
            if let Some(project_id) = cfg.project_id {
                gcloud_builder = gcloud_builder.project_id(project_id);
            }
            let layer = gcloud_builder.build().await?;
            builder.init_with_gcloud_telemetry(layer)?;
        }
        None => {
            builder.try_init()?;
        }
    }

    Ok(())
}
