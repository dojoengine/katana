use opentelemetry_gcloud_trace::errors::GcloudTraceError;
use tracing::subscriber::SetGlobalDefaultError;
use tracing_log::log::SetLoggerError;
use tracing_subscriber::filter;

mod builder;
mod fmt;
pub mod gcloud;
pub mod otlp;

pub use builder::TracingBuilder;
pub use fmt::LogFormat;

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
/// For new code, consider using [`TracingBuilder`] for more flexibility.
pub async fn init(format: LogFormat, telemetry_config: Option<TracerConfig>) -> Result<(), Error> {
    match format {
        LogFormat::Full => match telemetry_config {
            Some(TracerConfig::Otlp(cfg)) => {
                TracingBuilder::new().full().with_otlp(cfg).try_init().await
            }

            Some(TracerConfig::GCloud(cfg)) => {
                TracingBuilder::new().full().with_gcloud(cfg).try_init().await
            }

            None => TracingBuilder::new().full().try_init().await,
        },

        LogFormat::Json => match telemetry_config {
            Some(TracerConfig::Otlp(cfg)) => {
                TracingBuilder::new().json().with_otlp(cfg).try_init().await
            }

            Some(TracerConfig::GCloud(cfg)) => {
                TracingBuilder::new().json().with_gcloud(cfg).try_init().await
            }

            None => TracingBuilder::new().json().try_init().await,
        },
    }
}
