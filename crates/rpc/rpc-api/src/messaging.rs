use jsonrpsee::core::RpcResult;
use jsonrpsee::proc_macros::rpc;
use katana_rpc_types::messaging::MessagingCheckpoint;

/// Operator-facing RPC methods for the L1->L2 messaging server.
///
/// All three methods read or write the persisted messaging checkpoint AND
/// signal the running messenger to live-rewind its in-memory cursor, so
/// operators can recover missed messages without restarting the node.
#[cfg_attr(not(feature = "client"), rpc(server, namespace = "messaging"))]
#[cfg_attr(feature = "client", rpc(client, server, namespace = "messaging"))]
pub trait MessagingApi {
    /// Returns the last *committed* checkpoint — the same value the messaging
    /// server reads on boot. Returns `null` when no checkpoint row exists.
    ///
    /// This reflects the DB state, not the live in-memory gather position.
    #[method(name = "getCheckpoint")]
    async fn get_checkpoint(&self) -> RpcResult<Option<MessagingCheckpoint>>;

    /// Persist `(block, tx_index)` as the last processed message and rewind
    /// the live cursor to `(block, tx_index + 1)`.
    ///
    /// Note the off-by-one: a checkpoint represents the last successfully
    /// processed message, so the next gather resumes one past it. To re-gather
    /// from the very beginning of block 0 use [`resetCheckpoint`] instead —
    /// `setCheckpoint(0, 0)` would skip tx 0 of block 0.
    #[method(name = "setCheckpoint")]
    async fn set_checkpoint(&self, block: u64, tx_index: u64) -> RpcResult<()>;

    /// Delete the persisted checkpoint and rewind the live cursor to the
    /// messenger's configured `from_block` with `tx_index = 0`. The next boot
    /// will also start from `from_block` since no checkpoint row exists.
    #[method(name = "resetCheckpoint")]
    async fn reset_checkpoint(&self) -> RpcResult<()>;
}
