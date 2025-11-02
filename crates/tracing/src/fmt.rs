use std::fmt::Display;

use serde::{Deserialize, Serialize};
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::{self};
use tracing_subscriber::Layer;

/// Format for logging output.
#[derive(Debug, Copy, Clone, PartialEq, Deserialize, Serialize, Default, Eq)]
pub enum LogFormat {
    /// Full text format with colors and human-readable layout.
    #[default]
    Full,
    /// JSON format for structured logging, suitable for machine parsing.
    Json,
}

impl Display for LogFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json => write!(f, "json"),
            Self::Full => write!(f, "full"),
        }
    }
}

impl clap::ValueEnum for LogFormat {
    fn value_variants<'a>() -> &'a [Self] {
        &[Self::Json, Self::Full]
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        match self {
            Self::Json => Some(clap::builder::PossibleValue::new("json")),
            Self::Full => Some(clap::builder::PossibleValue::new("full")),
        }
    }
}

const DEFAULT_TIMESTAMP_FORMAT: &str = "%Y-%m-%d %H:%M:%S%.3f %Z";

/// Default formatter for span and event timestamps.
///
/// Formats timestamps in local time with the following format:
///
/// ```console
/// %Y-%m-%d %H:%M:%S%.3f %Z
/// ```
///
/// Example output
///
/// ```console
/// 2025-08-24 20:49:32.487 -04:00
/// ```
#[derive(Debug, Clone, Default)]
pub struct LocalTime;

impl LocalTime {
    pub fn new() -> Self {
        LocalTime
    }
}

impl time::FormatTime for LocalTime {
    fn format_time(&self, w: &mut Writer<'_>) -> std::fmt::Result {
        let time = chrono::Local::now();
        write!(w, "{}", time.format(DEFAULT_TIMESTAMP_FORMAT))
    }
}

// Use an enum to preserve type information instead of Box<dyn>
pub enum FmtLayer<F, J> {
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
