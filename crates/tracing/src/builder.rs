use std::marker::PhantomData;

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter, Layer};

use crate::fmt::LocalTime;
use crate::{gcloud, otlp, Error, LogFormat, TracerConfig};

const DEFAULT_LOG_FILTER: &str = "katana_db::mdbx=trace,cairo_native::compiler=off,pipeline=debug,\
                                  stage=debug,tasks=debug,executor=trace,forking::backend=trace,\
                                  blockifier=off,jsonrpsee_server=off,hyper=off,messaging=debug,\
                                  node=error,explorer=info,rpc=trace,pool=trace,info";

/// Identity type-state markers for [`TracingBuilder`].
#[derive(Debug)]
pub struct Identity;

/// [`TracingBuilder`] type-state markers for full log format [`LogFormat::Full`].
#[derive(Debug)]
pub struct FullFormat;
/// [`TracingBuilder`] type-state markers for JSON log format [`LogFormat::Json`].
#[derive(Debug)]
pub struct JsonFormat;

/// [`TracingBuilder`] type-state markers for OTLP tracer.
#[derive(Debug)]
pub struct OtlpTracer;
/// [`TracingBuilder`] type-state markers for GCloud tracer.
#[derive(Debug)]
pub struct GCloudTracer;

// Main builder struct with type-state for format
#[derive(Debug)]
pub struct TracingBuilder<Fmt = Identity, Telemetry = Identity> {
    service_name: String,
    log_format: Option<LogFormat>,
    filter: Option<EnvFilter>,
    tracer: Option<TracerConfig>,
    _format: PhantomData<Fmt>,
    _telemetry: PhantomData<Telemetry>,
}

impl TracingBuilder {
    /// Create a new tracing builder
    pub fn new() -> Self {
        Self {
            service_name: "katana".to_string(),
            log_format: None,
            filter: None,
            tracer: None,
            _format: PhantomData,
            _telemetry: PhantomData,
        }
    }
}

impl<Fmt, Telemetry> TracingBuilder<Fmt, Telemetry> {
    pub fn service_name<S: ToString>(mut self, service_name: S) -> Self {
        self.service_name = service_name.to_string();
        self
    }

    /// Set a custom filter from a string
    pub fn with_filter(mut self, filter: &str) -> Result<Self, Error> {
        self.filter = Some(EnvFilter::try_new(filter)?);
        Ok(self)
    }

    /// Use the default filter
    pub fn with_default_filter(mut self) -> Result<Self, Error> {
        self.filter = Some(EnvFilter::try_new(DEFAULT_LOG_FILTER)?);
        Ok(self)
    }

    /// Use filter from environment variable (RUST_LOG)
    pub fn with_env_filter(mut self) -> Result<Self, Error> {
        self.filter = Some(EnvFilter::try_from_default_env()?);
        Ok(self)
    }

    /// Use filter from environment with fallback to default
    pub fn with_env_filter_or_default(mut self) -> Result<Self, Error> {
        let default_filter = EnvFilter::try_new(DEFAULT_LOG_FILTER);
        self.filter = Some(EnvFilter::try_from_default_env().or(default_filter)?);
        Ok(self)
    }

    pub async fn try_init(self) -> Result<(), Error> {
        let filter = self.filter.unwrap_or_else(|| {
            EnvFilter::try_new(DEFAULT_LOG_FILTER).expect("default filter should be valid")
        });

        let log_format = self.log_format.unwrap_or(LogFormat::Full);

        let fmt = match log_format {
            LogFormat::Full => fmt::layer().with_timer(LocalTime::new()).boxed(),
            LogFormat::Json => fmt::layer().json().with_timer(LocalTime::new()).boxed(),
        };

        let registry = tracing_subscriber::registry().with(filter).with(fmt);

        match self.tracer {
            Some(TracerConfig::Otlp(cfg)) => {
                let tracer = otlp::init_tracer_with_service(&cfg, &self.service_name)?;
                let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);
                registry.with(telemetry).init();
            }
            Some(TracerConfig::GCloud(cfg)) => {
                let tracer = gcloud::init_tracer_with_service(&cfg, &self.service_name).await?;
                let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);
                registry.with(telemetry).init();
            }
            None => registry.init(),
        }

        Ok(())
    }

    pub async fn init(self) {
        self.try_init().await.expect("failed to initialize global tracer")
    }
}

impl<Telemetry> TracingBuilder<Identity, Telemetry> {
    /// Set the log format to full (human-readable with colors)
    pub fn full(self) -> TracingBuilder<FullFormat, Telemetry> {
        TracingBuilder {
            service_name: self.service_name,
            log_format: Some(LogFormat::Full),
            tracer: self.tracer,
            filter: self.filter,
            _format: PhantomData,
            _telemetry: PhantomData,
        }
    }

    /// Set the log format to JSON
    pub fn json(self) -> TracingBuilder<JsonFormat, Telemetry> {
        TracingBuilder {
            service_name: self.service_name,
            log_format: Some(LogFormat::Json),
            tracer: self.tracer,
            filter: self.filter,
            _format: PhantomData,
            _telemetry: PhantomData,
        }
    }
}

impl<Fmt> TracingBuilder<Fmt, Identity> {
    pub fn with_otlp(self, config: otlp::OtlpConfig) -> TracingBuilder<Fmt, OtlpTracer> {
        TracingBuilder {
            service_name: self.service_name,
            filter: self.filter,
            log_format: self.log_format,
            tracer: Some(TracerConfig::Otlp(config)),
            _format: PhantomData,
            _telemetry: PhantomData,
        }
    }

    pub fn with_gcloud(self, config: gcloud::GcloudConfig) -> TracingBuilder<Fmt, GCloudTracer> {
        TracingBuilder {
            service_name: self.service_name,
            filter: self.filter,
            log_format: self.log_format,
            tracer: Some(TracerConfig::GCloud(config)),
            _format: PhantomData,
            _telemetry: PhantomData,
        }
    }
}

impl Default for TracingBuilder {
    fn default() -> Self {
        Self::new()
    }
}
