use jsonrpsee::core::RpcResult;
use jsonrpsee::proc_macros::rpc;
use katana_primitives::block::BlockNumber;
use katana_rpc_types::broadcasted::{
    BroadcastedDeclareTx, BroadcastedDeployAccountTx, BroadcastedInvokeTx,
};
use katana_rpc_types::receipt::TxReceiptWithBlockInfo;
use katana_rpc_types::settlement::{BlockProof, SettlementStatus};

/// Katana-specific JSON-RPC methods.
#[cfg_attr(not(feature = "client"), rpc(server, namespace = "katana"))]
#[cfg_attr(feature = "client", rpc(client, server, namespace = "katana"))]
pub trait KatanaApi {
    /// Submit a new invoke transaction and wait until the receipt is available.
    ///
    /// This is a synchronous version of the `starknet_addInvokeTransaction` method where the
    /// request's response is the actual receipt of the transaction's execution - the receipt is
    /// returned immediately once it becomes available.
    #[method(name = "addInvokeTransactionSync")]
    async fn add_invoke_transaction_sync(
        &self,
        invoke_transaction: BroadcastedInvokeTx,
    ) -> RpcResult<TxReceiptWithBlockInfo>;

    /// Submit a new declare transaction and wait until the receipt is available.
    ///
    /// This is a synchronous version of the `starknet_addDeclareTransaction` method where the
    /// request's response is the actual receipt of the transaction's execution - the receipt is
    /// returned immediately once it becomes available.
    #[method(name = "addDeclareTransactionSync")]
    async fn add_declare_transaction_sync(
        &self,
        declare_transaction: BroadcastedDeclareTx,
    ) -> RpcResult<TxReceiptWithBlockInfo>;

    /// Submit a new deploy account transaction and wait until the receipt is available.
    ///
    /// This is a synchronous version of the `starknet_addDeployAccountTransaction` method where the
    /// request's response is the actual receipt of the transaction's execution - the receipt is
    /// returned immediately once it becomes available.
    #[method(name = "addDeployAccountTransactionSync")]
    async fn add_deploy_account_transaction_sync(
        &self,
        deploy_account_transaction: BroadcastedDeployAccountTx,
    ) -> RpcResult<TxReceiptWithBlockInfo>;
}

/// Settlement-related methods under the `katana` namespace.
///
/// Split into its own trait so the handler depends only on the settlement service's status —
/// keeping it decoupled from the main Starknet handler that serves the rest of the namespace.
#[cfg_attr(not(feature = "client"), rpc(server, namespace = "katana"))]
#[cfg_attr(feature = "client", rpc(client, server, namespace = "katana"))]
pub trait KatanaSettlementApi {
    /// Returns the status of the node's embedded settlement service: the most recent settled block
    /// and the current chain head.
    ///
    /// Always succeeds: on a node that does not settle, both are `0`.
    #[method(name = "settlementStatus")]
    async fn settlement_status(&self) -> RpcResult<SettlementStatus>;

    /// Returns the SP1 proof that settled the given block, identified by its Succinct
    /// prover-network request ID, or `null` if the block has not been settled with a network proof.
    #[method(name = "getBlockProof")]
    async fn get_block_proof(&self, block: BlockNumber) -> RpcResult<Option<BlockProof>>;
}
