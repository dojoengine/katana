use std::fmt::Display;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Format for logging output.
#[derive(Debug, Copy, Clone, PartialEq, Deserialize, Serialize, Default)]
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

impl FromStr for LogFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "json" => Ok(LogFormat::Json),
            "full" => Ok(LogFormat::Full),
            _ => Err(format!("invalid log format: '{s}'. Valid options are 'json' or 'full'")),
        }
    }
}

impl clap::ValueEnum for LogFormat {
    fn value_variants<'a>() -> &'a [Self] {
        &[Self::Json, Self::Full]
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        let value = self.to_string();
        Some(clap::builder::PossibleValue::new(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    #[case("json", LogFormat::Json)]
    #[case("full", LogFormat::Full)]
    #[case("JSON", LogFormat::Json)]
    #[case("FULL", LogFormat::Full)]
    #[case("Json", LogFormat::Json)]
    #[case("Full", LogFormat::Full)]
    fn log_format_from_str(#[case] input: &str, #[case] expected: LogFormat) {
        assert_eq!(LogFormat::from_str(input).unwrap(), expected);
    }

    #[rstest::rstest]
    #[case("invalid")]
    #[case("")]
    fn log_format_from_str_errors(#[case] input: &str) {
        assert!(LogFormat::from_str(input).is_err());
    }

    #[rstest::rstest]
    #[case(LogFormat::Json, "json")]
    #[case(LogFormat::Full, "full")]
    fn log_format_to_string(#[case] format: LogFormat, #[case] expected: &str) {
        assert_eq!(format.to_string(), expected);
    }
}
