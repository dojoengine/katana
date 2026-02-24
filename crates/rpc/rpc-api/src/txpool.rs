use jsonrpsee::core::RpcResult;
use jsonrpsee::proc_macros::rpc;
use katana_primitives::ContractAddress;
use katana_rpc_types::txpool::{TxPoolContent, TxPoolInspect, TxPoolStatus};

#[cfg_attr(not(feature = "client"), rpc(server, namespace = "txpool"))]
#[cfg_attr(feature = "client", rpc(client, server, namespace = "txpool"))]
pub trait TxPoolApi {
    #[method(name = "status")]
    async fn txpool_status(&self) -> RpcResult<TxPoolStatus>;

    #[method(name = "content")]
    async fn txpool_content(&self) -> RpcResult<TxPoolContent>;

    #[method(name = "contentFrom")]
    async fn txpool_content_from(&self, address: ContractAddress) -> RpcResult<TxPoolContent>;

    #[method(name = "inspect")]
    async fn txpool_inspect(&self) -> RpcResult<TxPoolInspect>;
}
