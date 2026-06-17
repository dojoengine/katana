use katana_primitives::block::BlockNumber;
use serde::{Deserialize, Serialize};

/// Status of the node's embedded settlement service, returned by `katana_settlementStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettlementStatus {
    /// The current local chain head.
    pub head: BlockNumber,
    /// The most recent block settled to the settlement chain.
    pub settled_block: BlockNumber,
}
