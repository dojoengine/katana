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
    #[method(name = "addInvokeTransaction")]
    async fn add_invoke_transaction(
        &self,
        invoke_transaction: BroadcastedInvokeTx,
    ) -> RpcResult<TxReceiptWithBlockInfo>;

    /// Submit a new declare transaction and wait until the receipt is available.
    #[method(name = "addDeclareTransaction")]
    async fn add_declare_transaction(
        &self,
        declare_transaction: BroadcastedDeclareTx,
    ) -> RpcResult<TxReceiptWithBlockInfo>;

    /// Submit a new deploy account transaction and wait until the receipt is available.
    #[method(name = "addDeployAccountTransaction")]
    async fn add_deploy_account_transaction(
        &self,
        deploy_account_transaction: BroadcastedDeployAccountTx,
    ) -> RpcResult<TxReceiptWithBlockInfo>;
}
