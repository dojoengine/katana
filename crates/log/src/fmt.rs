use std::fmt::Display;

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
