use opentelemetry_gcloud_trace::errors::GcloudTraceError;
use tracing::subscriber::SetGlobalDefaultError;
use tracing_log::log::SetLoggerError;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{filter, EnvFilter, Layer};

mod fmt;
pub mod gcloud;
pub mod otlp;

pub use fmt::LogFormat;

use crate::fmt::LocalTime;

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

pub async fn init(format: LogFormat, telemetry_config: Option<TracerConfig>) -> Result<(), Error> {
    const DEFAULT_LOG_FILTER: &str =
        "katana_db::mdbx=trace,cairo_native::compiler=off,pipeline=debug,stage=debug,tasks=debug,\
         executor=trace,forking::backend=trace,blockifier=off,jsonrpsee_server=off,hyper=off,\
         messaging=debug,node=error,explorer=info,rpc=trace,pool=trace,optimistic=debug,info";

    let default_filter = EnvFilter::try_new(DEFAULT_LOG_FILTER);
    let filter = EnvFilter::try_from_default_env().or(default_filter)?;

    // Initialize tracing subscriber with optional telemetry
    if let Some(telemetry_config) = telemetry_config {
        // Initialize telemetry layer based on exporter type
        let telemetry = match telemetry_config {
            TracerConfig::Gcloud(cfg) => {
                let tracer = gcloud::init_tracer(&cfg).await?;
                tracing_opentelemetry::layer().with_tracer(tracer)
            }
            TracerConfig::Otlp(cfg) => {
                let tracer = otlp::init_tracer(&cfg)?;
                tracing_opentelemetry::layer().with_tracer(tracer)
            }
        };

        let fmt = match format {
            LogFormat::Full => {
                tracing_subscriber::fmt::layer().with_timer(LocalTime::new()).boxed()
            }
            LogFormat::Json => {
                tracing_subscriber::fmt::layer().json().with_timer(LocalTime::new()).boxed()
            }
        };

        tracing_subscriber::registry().with(filter).with(telemetry).with(fmt).init();
    } else {
        let fmt = match format {
            LogFormat::Full => {
                tracing_subscriber::fmt::layer().with_timer(LocalTime::new()).boxed()
            }
            LogFormat::Json => {
                tracing_subscriber::fmt::layer().json().with_timer(LocalTime::new()).boxed()
            }
        };

        tracing_subscriber::registry().with(filter).with(fmt).init();
    }

    Ok(())
}
