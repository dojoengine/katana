use tracing::subscriber::SetGlobalDefaultError;
use tracing::Subscriber;
use tracing_log::log::SetLoggerError;
use tracing_log::LogTracer;
use tracing_subscriber::{filter, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

mod fmt;

pub use fmt::LogFormat;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to initialize log tracer: {0}")]
    LogTracerInit(#[from] SetLoggerError),

    #[error("failed to parse environment filter: {0}")]
    EnvFilterParse(#[from] filter::ParseError),

    #[error("failed to set global dispatcher: {0}")]
    SetGlobalDefault(#[from] SetGlobalDefaultError),
}

pub fn init(format: LogFormat, dev_log: bool) -> Result<(), Error> {
    const DEFAULT_LOG_FILTER: &str = "cairo_native::compiler=off,pipeline=debug,stage=debug,info,\
                                      tasks=debug,executor=trace,forking::backend=trace,\
                                      blockifier=off,jsonrpsee_server=off,hyper=off,\
                                      messaging=debug,node=error,explorer=info";

    let filter = if dev_log {
        format!("{DEFAULT_LOG_FILTER},server=debug")
    } else {
        DEFAULT_LOG_FILTER.to_string()
    };

    LogTracer::init()?;

    // If the user has set the `RUST_LOG` environment variable, then we prioritize it.
    // Otherwise, we use the default log filter.
    // TODO: change env var to `KATANA_LOG`.
    let filter = EnvFilter::try_from_default_env().or(EnvFilter::try_new(&filter))?;
    let builder = tracing_subscriber::fmt::Subscriber::builder().with_env_filter(filter);

    let subscriber: Box<dyn Subscriber + Send + Sync> = match format {
        LogFormat::Full => {
            let a = builder.finish();
            Box::new(a)
        }
        LogFormat::Json => Box::new(builder.json().finish()),
    };

    // tracing_subscriber::registry().with(layer).try_init();
    Ok(tracing::subscriber::set_global_default(subscriber)?)
}
