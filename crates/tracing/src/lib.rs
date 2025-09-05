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
    Gcloud(gcloud::GcloudConfig),
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
    let builder = TracingBuilder::new().with_log_format(format).with_env_filter_or_default()?;

    match telemetry_config {
        Some(TracerConfig::Otlp(cfg)) => {
            let mut otlp_builder = builder.otlp();
            if let Some(endpoint) = cfg.endpoint {
                otlp_builder = otlp_builder.with_endpoint(endpoint);
            }
            otlp_builder.build().await
        }
        Some(TracerConfig::Gcloud(cfg)) => {
            let mut gcloud_builder = builder.gcloud();
            if let Some(project_id) = cfg.project_id {
                gcloud_builder = gcloud_builder.with_project_id(project_id);
            }
            gcloud_builder.build().await
        }
        None => builder.build().await,
    }
}
