use std::path::PathBuf;
use std::sync::OnceLock;

use opentelemetry_gcloud_trace::errors::GcloudTraceError;
use tracing::subscriber::SetGlobalDefaultError;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_log::log::SetLoggerError;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{filter, EnvFilter, Layer};

mod fmt;
pub mod gcloud;
pub mod otlp;

pub use fmt::{LogColor, LogFormat};

use crate::fmt::LocalTime;

#[derive(Debug, Clone)]
pub struct LoggingConfig {
    /// Output format for the stdout sink.
    pub stdout_format: LogFormat,
    /// ANSI color policy for the stdout sink (independent of file output).
    pub stdout_color: LogColor,

    /// Enables the file sink when `true`; when `false`, only stdout logging is used.
    pub file_enabled: bool,
    /// Output format for the file sink.
    pub file_format: LogFormat,
    /// Directory where rotated log files are written when the file sink is enabled.
    pub file_directory: PathBuf,
    /// Maximum number of rotated log files to keep (0 means unlimited).
    pub file_max_files: usize,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            stdout_format: LogFormat::Full,
            stdout_color: LogColor::Always,
            file_enabled: false,
            file_format: LogFormat::Full,
            file_directory: default_log_file_directory(),
            file_max_files: 0,
        }
    }
}

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

    #[error("log file io error: {0}")]
    LogFileIo(#[from] std::io::Error),

    #[error("failed to initialize rolling file appender: {0}")]
    RollingFileInit(#[from] tracing_appender::rolling::InitError),

    #[error("google cloud trace error: {0}")]
    GcloudTrace(#[from] GcloudTraceError),

    #[error("failed to install crypto provider")]
    InstallCryptoFailed,

    #[error("failed to build otlp tracer: {0}")]
    OtlpBuild(#[from] opentelemetry_otlp::ExporterBuildError),

    #[error(transparent)]
    OtelSdk(#[from] opentelemetry_sdk::error::OTelSdkError),
}

/// Keep `tracing_appender::non_blocking` workers alive for the lifetime of the process.
///
/// `tracing_appender::non_blocking()` returns a `WorkerGuard` that must be held; if it is dropped
/// the background worker stops and buffered log lines may never be written.
static LOG_FILE_GUARD: OnceLock<tracing_appender::non_blocking::WorkerGuard> = OnceLock::new();

pub async fn init(
    logging: LoggingConfig,
    telemetry_config: Option<TracerConfig>,
) -> Result<(), Error> {
    const DEFAULT_LOG_FILTER: &str =
        "katana_db::mdbx=trace,cairo_native::compiler=off,pipeline=debug,stage=debug,tasks=debug,\
         executor=trace,forking::backend=trace,blockifier=off,jsonrpsee_server=off,hyper=off,\
         messaging=debug,node=error,explorer=info,rpc=trace,pool=trace,\
         katana_stage::downloader=trace,katana_paymaster=trace,info";

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

        let stdout_layer = stdout_layer(&logging);
        let file_layer = file_layer(&logging)?;

        tracing_subscriber::registry()
            .with(filter)
            .with(telemetry)
            .with(stdout_layer)
            .with(file_layer)
            .init();
    } else {
        let stdout_layer = stdout_layer(&logging);
        let file_layer = file_layer(&logging)?;
        tracing_subscriber::registry().with(filter).with(stdout_layer).with(file_layer).init();
    }

    Ok(())
}

/// Returns the default directory where the log files will be stored at.
pub fn default_log_file_directory() -> PathBuf {
    dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("katana").join("logs")
}

fn stdout_layer<S>(cfg: &LoggingConfig) -> Box<dyn Layer<S> + Send + Sync>
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    let ansi = match cfg.stdout_color {
        LogColor::Always => true,
        LogColor::Never => false,
        LogColor::Auto => std::io::IsTerminal::is_terminal(&std::io::stdout()),
    };

    match cfg.stdout_format {
        LogFormat::Full => {
            tracing_subscriber::fmt::layer().with_timer(LocalTime::new()).with_ansi(ansi).boxed()
        }

        LogFormat::Json => tracing_subscriber::fmt::layer()
            .json()
            .with_timer(LocalTime::new())
            .with_ansi(false)
            .boxed(),
    }
}

fn file_layer<S>(cfg: &LoggingConfig) -> Result<Option<Box<dyn Layer<S> + Send + Sync>>, Error>
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    if !cfg.file_enabled {
        return Ok(None);
    };

    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_suffix("katana.log")
        .max_log_files(cfg.file_max_files)
        .build(&cfg.file_directory)?;

    let (non_blocking, guard) = tracing_appender::non_blocking(appender);
    let _ = LOG_FILE_GUARD.set(guard);

    Ok(Some(match cfg.file_format {
        LogFormat::Full => tracing_subscriber::fmt::layer()
            .with_timer(LocalTime::new())
            .with_writer(non_blocking)
            .with_ansi(false)
            .boxed(),

        LogFormat::Json => tracing_subscriber::fmt::layer()
            .json()
            .with_timer(LocalTime::new())
            .with_writer(non_blocking)
            .with_ansi(false)
            .boxed(),
    }))
}
