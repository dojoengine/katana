use std::fmt::Debug;

use tracing_subscriber::fmt::format::{DefaultFields, Format, Full, Json, JsonFields};
use tracing_subscriber::layer::{Layered, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter, Registry};

use crate::fmt::{FmtLayer, LocalTime};
use crate::{Error, LogFormat, TelemetryTracer};

const DEFAULT_LOG_FILTER: &str = "katana_db::mdbx=trace,cairo_native::compiler=off,pipeline=debug,\
                                  stage=debug,tasks=debug,executor=trace,forking::backend=trace,\
                                  blockifier=off,jsonrpsee_server=off,hyper=off,messaging=debug,\
                                  node=error,explorer=info,rpc=trace,pool=trace,info";

pub type NoopTracer = opentelemetry::trace::noop::NoopTracer;

#[derive(Debug)]
pub struct TracingBuilder<Telemetry = NoopTracer> {
    filter: Option<EnvFilter>,
    log_format: LogFormat,
    tracer: Telemetry,
}

impl TracingBuilder {
    /// Create a new tracing builder
    pub fn new() -> Self {
        Self { filter: None, log_format: LogFormat::Full, tracer: NoopTracer::new() }
    }
}

impl TracingBuilder<NoopTracer> {
    pub fn with_telemetry<T: TelemetryTracer>(self, tracer: T) -> TracingBuilder<T> {
        TracingBuilder { filter: self.filter, log_format: self.log_format, tracer }
    }
}

impl<Telemetry> TracingBuilder<Telemetry> {
    /// Set the log format to JSON
    pub fn json(self) -> TracingBuilder<Telemetry> {
        TracingBuilder { log_format: LogFormat::Json, tracer: self.tracer, filter: self.filter }
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
}

impl<Telemetry: TelemetryTracer> TracingBuilder<Telemetry> {
    /// Try to initialize the tracing subscriber without telemetry
    pub fn build(self) -> Result<TracingSubscriber<Telemetry>, Error> {
        let filter = self.filter.unwrap_or_else(|| {
            EnvFilter::try_new(DEFAULT_LOG_FILTER).expect("default filter should be valid")
        });

        let base_layer = fmt::layer().with_timer(LocalTime::new());

        let fmt_layer = match self.log_format {
            LogFormat::Full => FmtLayer::Full(base_layer),
            LogFormat::Json => FmtLayer::Json(base_layer.json()),
        };

        Ok(TracingSubscriber {
            tracer: self.tracer,
            subscriber: tracing_subscriber::registry().with(filter).with(fmt_layer),
        })
    }
}

impl Default for TracingBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// The base subscribe type created by [`TracingBuilder`] and used by [`TracingSubscriber`].
type BaseSubscriber = Layered<
    FmtLayer<
        fmt::Layer<Layered<EnvFilter, Registry>, DefaultFields, Format<Full, LocalTime>>,
        fmt::Layer<Layered<EnvFilter, Registry>, JsonFields, Format<Json, LocalTime>>,
    >,
    Layered<EnvFilter, Registry>,
>;

pub struct TracingSubscriber<Telemetry> {
    subscriber: BaseSubscriber,
    tracer: Telemetry,
}

impl<Telemetry: TelemetryTracer> TracingSubscriber<Telemetry> {
    pub fn init(self) {
        self.tracer.init().unwrap();
        self.subscriber.with(tracing_opentelemetry::layer().with_tracer(self.tracer)).init();
    }
}

impl<Telemetry: Debug> Debug for TracingSubscriber<Telemetry> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TracingSubscriber")
            .field("subscriber", &"..")
            .field("tracer", &self.tracer)
            .finish()
    }
}
