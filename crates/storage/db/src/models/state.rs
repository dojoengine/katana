use katana_primitives::block::BlockNumber;
use serde::{Deserialize, Serialize};

/// Provider-owned retention watermark for historical state access.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[cfg_attr(test, derive(::arbitrary::Arbitrary))]
pub struct HistoricalStateRetention {
    /// The first block for which historical state is still available.
    pub earliest_available_block: BlockNumber,
}
