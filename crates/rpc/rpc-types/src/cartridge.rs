use serde::{Deserialize, Serialize};

/// Source of fees for cartridge outside execution requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeeSource {
    Paymaster,
    Credits,
}
