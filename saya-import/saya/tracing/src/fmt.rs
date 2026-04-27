use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time;

const DEFAULT_TIMESTAMP_FORMAT: &str = "%Y-%m-%d %H:%M:%S%.3f %Z";

/// Default formatter for span and event timestamps.
///
/// Formats timestamps in local time with the following format:
///
/// ```console
/// %Y-%m-%d %H:%M:%S%.3f %Z
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
