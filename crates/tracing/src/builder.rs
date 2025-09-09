use std::marker::PhantomData;

use opentelemetry::trace::noop::NoopTracer;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::fmt::format::{self};
use tracing_subscriber::layer::{Layered, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter, Layer, Registry};

use crate::fmt::LocalTime;
use crate::{Error, GCloudTracingBuilder, LogFormat, TelemetryTracer};

type DefaultFormat = format::Full;

type Subscriber<Tracer> = Layered<
    OpenTelemetryLayer<
        Layered<
            Box<dyn Layer<Layered<EnvFilter, Registry>> + Send + Sync + 'static>,
            Layered<EnvFilter, Registry>,
        >,
        Tracer,
    >,
    Layered<
        Box<dyn Layer<Layered<EnvFilter, Registry>> + Send + Sync + 'static>,
        Layered<EnvFilter, Registry>,
    >,
>;

struct TracingSubscriber<Fmt, Tracer> {
    subscriber: Subscriber<Tracer>,
    _fmt: PhantomData<Fmt>,
}

impl<Fmt, Tracer: TelemetryTracer> TracingSubscriber<Fmt, Tracer> {
    fn init(self) {
        self.subscriber.init();
    }
}

const DEFAULT_LOG_FILTER: &str = "katana_db::mdbx=trace,cairo_native::compiler=off,pipeline=debug,\
                                  stage=debug,tasks=debug,executor=trace,forking::backend=trace,\
                                  blockifier=off,jsonrpsee_server=off,hyper=off,messaging=debug,\
                                  node=error,explorer=info,rpc=trace,pool=trace,info";

// /// Identity type-state markers for [`TracingBuilder`].
// #[derive(Debug)]
// pub struct Identity;

// Main builder struct with type-state for format
#[derive(Debug)]
pub struct TracingBuilder<Fmt = format::Full, Telemetry = NoopTracer> {
    log_format: LogFormat,
    filter: Option<EnvFilter>,
    tracer: Telemetry,
    _format: PhantomData<Fmt>,
}

impl TracingBuilder {
    /// Create a new tracing builder
    pub fn new() -> Self {
        Self {
            filter: None,
            log_format: LogFormat::Full,
            tracer: NoopTracer::new(),
            _format: PhantomData,
        }
    }
}

impl<Fmt> TracingBuilder<Fmt, NoopTracer> {
    pub fn with_telemetry<T: TelemetryTracer>(self, tracer: T) -> TracingBuilder<Fmt, T> {
        TracingBuilder {
            filter: self.filter,
            log_format: self.log_format,
            tracer,
            _format: PhantomData,
        }
    }
}

impl<Fmt, Telemetry> TracingBuilder<Fmt, Telemetry> {
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

impl<Fmt, Telemetry: TelemetryTracer> TracingBuilder<Fmt, Telemetry> {
    /// Try to initialize the tracing subscriber without telemetry
    pub fn build(self) -> Result<TracingSubscriber<Fmt, Telemetry>, Error> {
        let filter = self.filter.unwrap_or_else(|| {
            EnvFilter::try_new(DEFAULT_LOG_FILTER).expect("default filter should be valid")
        });

        let fmt_layer = fmt::layer().with_timer(LocalTime::new());
        let fmt_layer = match self.log_format {
            LogFormat::Full => fmt_layer.boxed(),
            LogFormat::Json => fmt_layer.json().boxed(),
        };

        let telem = tracing_opentelemetry::layer().with_tracer(self.tracer);
        let subscriber = tracing_subscriber::registry().with(filter).with(fmt_layer).with(telem);

        Ok(TracingSubscriber { subscriber, _fmt: PhantomData })
    }
}

impl<Telemetry> TracingBuilder<DefaultFormat, Telemetry> {
    /// Set the log format to JSON
    pub fn json(self) -> TracingBuilder<format::Json, Telemetry> {
        TracingBuilder {
            log_format: LogFormat::Json,
            tracer: self.tracer,
            filter: self.filter,
            _format: PhantomData,
        }
    }
}

impl Default for TracingBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[tokio::test]
async fn foo() {
    let builder = TracingBuilder::new().build().unwrap();

    let gcloud = GCloudTracingBuilder::new().build().await.unwrap();
    let builder = TracingBuilder::new().json().with_telemetry(gcloud).build().unwrap();
}
