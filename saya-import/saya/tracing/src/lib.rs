use std::io::IsTerminal;

use tracing::subscriber::SetGlobalDefaultError;
use tracing_log::log::SetLoggerError;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{filter, EnvFilter};

mod fmt;

pub use fmt::LocalTime;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to initialize log tracer: {0}")]
    LogTracerInit(#[from] SetLoggerError),

    #[error("failed to parse environment filter: {0}")]
    EnvFilterParse(#[from] filter::ParseError),

    #[error("failed to set global dispatcher: {0}")]
    SetGlobalDefault(#[from] SetGlobalDefaultError),
}

/// Installs the process-wide tracing subscriber.
///
/// Reads the env filter from `RUST_LOG`; falls back to `default_filter` if unset.
/// Enables ANSI colors when stdout is a terminal. Bridges `log` crate records from
/// dependencies into the tracing subscriber.
pub fn init(default_filter: &str) -> Result<(), Error> {
    let filter =
        EnvFilter::try_from_default_env().or_else(|_| EnvFilter::try_new(default_filter))?;

    let stdout_layer = tracing_subscriber::fmt::layer()
        .with_timer(LocalTime::new())
        .with_ansi(std::io::stdout().is_terminal());

    tracing_subscriber::registry()
        .with(filter)
        .with(stdout_layer)
        .init();

    Ok(())
}
