use jsonrpsee::core::RpcResult;
use jsonrpsee::proc_macros::rpc;
use katana_rpc_types::node::NodeInfo;

/// Methods for introspecting a running Katana node.
#[cfg_attr(not(feature = "client"), rpc(server, namespace = "node"))]
#[cfg_attr(feature = "client", rpc(client, server, namespace = "node"))]
pub trait NodeApi {
    /// Returns the node's identity and build information.
    #[method(name = "getInfo")]
    async fn get_info(&self) -> RpcResult<NodeInfo>;
}
