use jsonrpsee::core::RpcResult;
use jsonrpsee::proc_macros::rpc;
use katana_primitives::ContractAddress;
use katana_rpc_types::txpool::{TxPoolContent, TxPoolInspect, TxPoolStatus};

/// Inspection API for the node's local transaction pool.
///
/// This exposes the node's own pending-transaction pool — not a network-wide mempool.
/// Unlike Ethereum, Starknet does not yet have a shared peer-to-peer mempool; each sequencer
/// maintains its own pool of transactions waiting to be included in a block. The
/// transactions visible here are only those that have been submitted to *this* node.
///
/// This API is primarily intended for debugging and diagnostics.
///
/// Modeled after Ethereum's `txpool_*` namespace, adapted for Starknet transactions.
///
/// All responses distinguish between `pending` (ready to execute) and `queued` (waiting on
/// a nonce gap) transactions. Currently Katana has no queued pool — dependent transactions
/// are rejected at submission — so the `queued` fields are always empty/zero. They exist
/// for forward-compatibility.
#[cfg_attr(not(feature = "client"), rpc(server, namespace = "txpool"))]
#[cfg_attr(feature = "client", rpc(client, server, namespace = "txpool"))]
pub trait TxPoolApi {
    /// Returns the number of pending and queued transactions in the pool.
    ///
    /// This is a cheap call that avoids snapshotting individual transactions.
    #[method(name = "status")]
    async fn txpool_status(&self) -> RpcResult<TxPoolStatus>;

    /// Returns the full content of the pool, grouped by sender address and nonce.
    ///
    /// Each transaction is represented as a lightweight [`TxPoolTransaction`] containing
    /// the hash, nonce, sender, max_fee, and tip. The full transaction can be retrieved
    /// via `starknet_getTransactionByHash`.
    #[method(name = "content")]
    async fn txpool_content(&self) -> RpcResult<TxPoolContent>;

    /// Same as `txpool_content` but filtered to a single sender address.
    ///
    /// Returns only the transactions from the given address. The `queued` map is always empty.
    #[method(name = "contentFrom")]
    async fn txpool_content_from(&self, address: ContractAddress) -> RpcResult<TxPoolContent>;

    /// Returns a textual summary of all pooled transactions, grouped by sender and nonce.
    ///
    /// Each transaction is represented as a human-readable string
    /// (`hash=0x… nonce=0x… max_fee=… tip=…`) rather than a structured object —
    /// useful for quick inspection or logging.
    #[method(name = "inspect")]
    async fn txpool_inspect(&self) -> RpcResult<TxPoolInspect>;
}
