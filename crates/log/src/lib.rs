use opentelemetry_gcloud_trace::errors::GcloudTraceError;
use tracing::subscriber::SetGlobalDefaultError;
use tracing_log::log::SetLoggerError;
use tracing_log::LogTracer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{filter, EnvFilter, Layer};

mod fmt;
pub mod gcloud;
pub mod otel;

pub use fmt::LogFormat;

#[derive(Debug, Clone)]
pub enum TracerConfig {
    Gcloud(gcloud::GcloudConfig),
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

    #[error("failed to install crypto provider")]
    InstallCryptoFailed,

    #[error(transparent)]
    OtelSdk(#[from] opentelemetry_sdk::error::OTelSdkError),
}

pub async fn init(
    format: LogFormat,
    dev_log: bool,
    telemetry_config: Option<TracerConfig>,
) -> Result<(), Error> {
    const DEFAULT_LOG_FILTER: &str =
        "cairo_native::compiler=off,pipeline=debug,stage=debug,info,tasks=debug,executor=trace,\
         forking::backend=trace,blockifier=off,jsonrpsee_server=off,hyper=off,messaging=debug,\
         node=error,explorer=info,jsonrpsee_core::middleware::layer::logger=trace";

    let filter = if dev_log {
        format!("{DEFAULT_LOG_FILTER},server=debug")
    } else {
        DEFAULT_LOG_FILTER.to_string()
    };

    // If the user has set the `RUST_LOG` environment variable, then we prioritize it.
    // Otherwise, we use the default log filter.
    // TODO: change env var to `KATANA_LOG`.
    let filter = EnvFilter::try_from_default_env().or(EnvFilter::try_new(&filter))?;

    // Initialize tracing subscriber with optional telemetry
    if let Some(telemetry_config) = telemetry_config {
        // Initialize telemetry layer based on exporter type
        let telemetry = match telemetry_config {
            TracerConfig::Gcloud(cfg) => {
                let tracer = gcloud::init_gcp_tracer(&cfg).await?;
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
