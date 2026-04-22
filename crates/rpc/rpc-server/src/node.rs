use jsonrpsee::core::{async_trait, RpcResult};
use katana_rpc_api::node::NodeApiServer;
use katana_rpc_types::node::NodeInfo;

#[derive(Debug, Clone)]
pub struct NodeApi {
    info: NodeInfo,
}

impl NodeApi {
    pub fn new(info: NodeInfo) -> Self {
        Self { info }
    }
}

#[async_trait]
impl NodeApiServer for NodeApi {
    async fn get_info(&self) -> RpcResult<NodeInfo> {
        Ok(self.info.clone())
    }
}

#[cfg(test)]
mod tests {
    use katana_chain_spec::ChainSpec;
    use katana_rpc_api::node::NodeApiServer;
    use katana_rpc_types::node::{ChainKind, NodeInfo};

    use super::NodeApi;

    fn sample_info() -> NodeInfo {
        NodeInfo {
            version: "1.2.3".into(),
            git_sha: "deadbee".into(),
            build_timestamp: "2026-04-21T00:00:00Z".into(),
            features: vec!["native".into()],
            chain_id: ChainSpec::dev().id().id(),
            chain_kind: ChainKind::Sequencer,
            dev: true,
        }
    }

    #[tokio::test]
    async fn get_info_returns_configured_info() {
        let info = sample_info();
        let api = NodeApi::new(info.clone());
        assert_eq!(api.get_info().await.unwrap(), info);
    }
}
