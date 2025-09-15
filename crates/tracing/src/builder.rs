use std::fmt::Debug;
use std::marker::PhantomData;

// use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::fmt::format::{self};
use tracing_subscriber::layer::{Layered, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter, Layer, Registry};

use crate::fmt::LocalTime;
use crate::{Error, LogFormat, TelemetryTracer};

const DEFAULT_LOG_FILTER: &str = "katana_db::mdbx=trace,cairo_native::compiler=off,pipeline=debug,\
                                  stage=debug,tasks=debug,executor=trace,forking::backend=trace,\
                                  blockifier=off,jsonrpsee_server=off,hyper=off,messaging=debug,\
                                  node=error,explorer=info,rpc=trace,pool=trace,info";

pub type NoopTracer = opentelemetry::trace::noop::NoopTracer;

// Format trait markers
pub type DefaultFormat = format::Full;
pub type Full = format::Full;
pub type Json = format::Json;

// type Subscriber<Telemetry> = Layered<
//     OpenTelemetryLayer<
//         Layered<
//             Box<dyn Layer<Layered<EnvFilter, Registry>> + Send + Sync + 'static>,
//             Layered<EnvFilter, Registry>,
//         >,
//         Telemetry,
//     >,
//     Layered<
//         Box<dyn Layer<Layered<EnvFilter, Registry>> + Send + Sync + 'static>,
//         Layered<EnvFilter, Registry>,
//     >,
// >;

type SubscriberWithNoTelemetry = Layered<
    Box<dyn Layer<Layered<EnvFilter, Registry>> + Send + Sync + 'static>,
    Layered<EnvFilter, Registry>,
>;

// #[derive(Clone)]
struct TracingSubscriber<Fmt, Telemetry> {
    subscriber_without_telem: SubscriberWithNoTelemetry,
    tracer: Telemetry,
    _fmt: PhantomData<Fmt>,
}

impl<Fmt, Telemetry: TelemetryTracer> TracingSubscriber<Fmt, Telemetry> {
    pub fn init(self) {
        self.tracer.init().unwrap();

        let telem = tracing_opentelemetry::layer().with_tracer(self.tracer);
        self.subscriber_without_telem.with(telem).init();
    }
}

impl<Fmt, Telemetry: Debug> Debug for TracingSubscriber<Fmt, Telemetry> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TracingSubscriber")
            .field("subscriber", &"..")
            .field("tracer", &self.tracer)
            .finish()
    }
}

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

        let base_layer = fmt::layer().with_timer(LocalTime::new());

        // Use an enum to preserve type information instead of Box<dyn>
        enum FmtLayer<F, J> {
            Full(F),
            Json(J),
        }

        impl<S, F, J> Layer<S> for FmtLayer<F, J>
        where
            S: tracing::Subscriber,
            F: Layer<S>,
            J: Layer<S>,
        {
            fn on_layer(&mut self, subscriber: &mut S) {
                match self {
                    FmtLayer::Full(layer) => layer.on_layer(subscriber),
                    FmtLayer::Json(layer) => layer.on_layer(subscriber),
                }
            }
        }

        let fmt_layer = match self.log_format {
            LogFormat::Full => FmtLayer::Full(base_layer),
            LogFormat::Json => FmtLayer::Json(base_layer.json()),
        };

        // let telem = tracing_opentelemetry::layer().with_tracer(self.tracer);
        let subscriber = tracing_subscriber::registry().with(filter).with(fmt_layer);

        Ok(TracingSubscriber {
            tracer: self.tracer,
            subscriber_without_telem: subscriber,
            _fmt: PhantomData,
        })
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
    use crate::{GCloudTracerBuilder, OtlpTracerBuilder};

    let builder = TracingBuilder::new().build().unwrap();

    let oltp = OtlpTracerBuilder::new().build().unwrap();
    let gcloud = GCloudTracerBuilder::new().build().await.unwrap();

    let builder_w_otlp = TracingBuilder::new().json().with_telemetry(oltp).build().unwrap();
    let builder_w_gcloud = TracingBuilder::new().json().with_telemetry(gcloud).build().unwrap();
}
