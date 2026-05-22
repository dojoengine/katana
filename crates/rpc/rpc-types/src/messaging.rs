use serde::{Deserialize, Serialize};

/// RPC representation of the messaging service checkpoint.
///
/// Mirrors `katana_provider_api::messaging::MessagingCheckpoint` but lives in
/// the RPC type crate so the public wire format stays independent of provider
/// internals. The `From` conversion lives in `katana-rpc-server` to avoid a
/// `rpc-types -> provider-api` dependency cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagingCheckpoint {
    /// The settlement chain block number last successfully processed.
    pub block: u64,
    /// The transaction index within `block` up to which messages have been
    /// processed.
    pub tx_index: u64,
}
