use jsonrpsee::core::RpcResult;
use jsonrpsee::proc_macros::rpc;
use katana_rpc_types::broadcasted::{
    BroadcastedDeclareTx, BroadcastedDeployAccountTx, BroadcastedInvokeTx,
};
use katana_rpc_types::receipt::TxReceiptWithBlockInfo;

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
