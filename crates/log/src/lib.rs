use std::fs::{File, OpenOptions};
use std::io::stdout;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::sync::Arc;

use opentelemetry_gcloud_trace::errors::GcloudTraceError;
use tracing::subscriber::SetGlobalDefaultError;
use tracing_log::log::SetLoggerError;
use tracing_subscriber::fmt::MakeWriter;
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

#[derive(Debug, Default, Clone)]
pub enum LogOutput {
    #[default]
    Stdout,
    File(PathBuf),
}

#[derive(Debug)]

pub struct Logger {}

trait LoggerOutput {
    type Writer: for<'writer> MakeWriter<'writer> + 'static;
    fn writer(&self) -> Self::Writer;
}

struct Stdout;

struct FileOutput {
    path: PathBuf,
}

impl LoggerOutput for Stdout {
    type Writer = Arc<std::io::Stdout>;
    fn writer(&self) -> Self::Writer {
        Arc::new(std::io::stdout())
    }
}
impl LoggerOutput for FileOutput {
    type Writer = File;
    fn writer(&self) -> Self::Writer {
        OpenOptions::new().create(true).append(true).open(&self.path).unwrap()
    }
}

#[derive(Debug)]
pub struct LoggerBuilder<W = Stdout> {
    dev: bool,
    format: LogFormat,
    filter: Option<String>,
    writer: W,
    telemetry_config: Option<TracerConfig>,
}

impl LoggerBuilder {
    pub fn new() -> Self {
        Self {
            dev: false,
            filter: None,
            writer: Stdout,
            telemetry_config: None,
            format: LogFormat::Full,
        }
    }
}

impl<W> LoggerBuilder<W> {
    pub fn with_writer<W2: LoggerOutput>(self, writer: W2) -> LoggerBuilder<W2> {
        LoggerBuilder {
            writer,
            dev: self.dev,
            format: self.format,
            filter: self.filter,
            telemetry_config: self.telemetry_config,
        }
    }

    pub fn file<P: Into<PathBuf>>(self, path: P) -> LoggerBuilder<FileOutput> {
        LoggerBuilder {
            dev: self.dev,
            format: self.format,
            filter: self.filter,
            telemetry_config: self.telemetry_config,
            writer: FileOutput { path: path.into() },
        }
    }

    pub fn format(mut self, fmt: LogFormat) -> Self {
        self.format = fmt;
        self
    }

    pub fn dev(mut self, enabled: bool) -> Self {
        self.dev = enabled;
        self
    }

    pub fn telemetry(mut self, config: TracerConfig) -> Self {
        self.telemetry_config = Some(config);
        self
    }

    pub fn filter<S: Into<String>>(mut self, filter: S) -> Self {
        self.filter = Some(filter.into());
        self
    }

    // pub async fn init(self) -> Result<(), Error> {
    //     let default_filter = if self.dev {
    //         format!("{DEFAULT_LOG_FILTER},server=debug")
    //     } else {
    //         DEFAULT_LOG_FILTER.to_string()
    //     };

    //     let filter_str = self.filter.clone().unwrap_or(default_filter);
    //     let filter = EnvFilter::try_from_default_env().or(EnvFilter::try_new(&filter_str))?;

    //     match self.writer {
    //         LogOutput::None => {
    //             tracing_subscriber::registry().with(filter).init();
    //             return Ok(());
    //         }
    //         LogOutput::Stdout => {
    //             self.init_with_stdout_output(filter).await?;
    //         }
    //         LogOutput::File(ref path) => {
    //             self.init_with_file_output(filter, path.to_path_buf()).await?;
    //         }
    //     }

    //     Ok(())
    // }

    // async fn init_with_stdout_output(self, filter: EnvFilter) -> Result<(), Error> {
    //     if let Some(telemetry_config) = self.telemetry_config {
    //         let telemetry = match telemetry_config {
    //             TracerConfig::Gcloud(cfg) => {
    //                 let tracer = gcloud::init_gcp_tracer(&cfg).await?;
    //                 tracing_opentelemetry::layer().with_tracer(tracer)
    //             }
    //         };

    //         let fmt = match self.format {
    //             LogFormat::Full => tracing_subscriber::fmt::layer().boxed(),
    //             LogFormat::Json => {
    //                 let a = tracing_subscriber::fmt::layer().json();
    //                 a.boxed()
    //             }
    //         };

    //         tracing_subscriber::registry().with(filter).with(telemetry).with(fmt).init();
    //     } else {
    //         let fmt = match self.format {
    //             LogFormat::Full => tracing_subscriber::fmt::layer().boxed(),
    //             LogFormat::Json => tracing_subscriber::fmt::layer().json().boxed(),
    //         };

    //         tracing_subscriber::registry().with(filter).with(fmt).init();
    //     }

    //     Ok(())
    // }

    // async fn init_with_file_output(self, filter: EnvFilter, path: PathBuf) -> Result<(), Error> {
    //     let file = std::fs::OpenOptions::new()
    //         .create(true)
    //         .append(true)
    //         .open(&path)
    //         .map_err(|e| Error::FileOpen(path.clone(), e))?;

    //     if let Some(telemetry_config) = self.telemetry_config {
    //         let telemetry = match telemetry_config {
    //             TracerConfig::Gcloud(cfg) => {
    //                 let tracer = gcloud::init_gcp_tracer(&cfg).await?;
    //                 tracing_opentelemetry::layer().with_tracer(tracer)
    //             }
    //         };

    //         let fmt = match self.format {
    //             LogFormat::Full => {
    //                 tracing_subscriber::fmt::layer().with_writer(Arc::new(stdout())).boxed()
    //             }
    //             LogFormat::Json => {
    //                 tracing_subscriber::fmt::layer().json().with_writer(file).boxed()
    //             }
    //         };

    //         tracing_subscriber::registry().with(filter).with(telemetry).with(fmt).init();
    //     } else {
    //         let fmt = match self.format {
    //             LogFormat::Full => tracing_subscriber::fmt::layer().with_writer(file).boxed(),
    //             LogFormat::Json => {
    //                 tracing_subscriber::fmt::layer().json().with_writer(file).boxed()
    //             }
    //         };

    //         tracing_subscriber::registry().with(filter).with(fmt).init();
    //     }

    //     Ok(())
    // }
}

impl<W: LoggerOutput> LoggerBuilder<W> {
    pub fn build(self) {
        todo!()
    }
}

const DEFAULT_LOG_FILTER: &str =
    "cairo_native::compiler=off,pipeline=debug,stage=debug,info,tasks=debug,executor=trace,\
     forking::backend=trace,blockifier=off,jsonrpsee_server=off,hyper=off,messaging=debug,\
     node=error,explorer=info,jsonrpsee_core::middleware::layer::logger=trace";

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

    #[error("failed to open file {0}: {1}")]
    FileOpen(PathBuf, std::io::Error),

    #[error(transparent)]
    OtelSdk(#[from] opentelemetry_sdk::error::OTelSdkError),
}

pub async fn init(
    format: LogFormat,
    dev_log: bool,
    telemetry_config: Option<TracerConfig>,
) -> Result<(), Error> {
    let mut builder = LoggerBuilder::new().format(format).dev(dev_log);

    if let Some(config) = telemetry_config {
        builder = builder.telemetry(config);
    }

    builder.init().await
}
