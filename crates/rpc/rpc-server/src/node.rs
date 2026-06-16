use jsonrpsee::core::{async_trait, RpcResult};
use jsonrpsee::types::ErrorObjectOwned;
use katana_rpc_api::node::NodeApiServer;
use katana_rpc_types::node::{NodeConfig, NodeInfo};

#[derive(Debug, Clone)]
pub struct NodeApi {
    info: NodeInfo,
    /// The node's runtime configuration. `None` for full nodes, which don't report a
    /// sequencer-shaped config (see `get_config`).
    config: Option<NodeConfig>,
}

impl NodeApi {
    pub fn new(info: NodeInfo, config: Option<NodeConfig>) -> Self {
        Self { info, config }
    }
}

#[async_trait]
impl NodeApiServer for NodeApi {
    async fn get_info(&self) -> RpcResult<NodeInfo> {
        Ok(self.info.clone())
    }

    async fn get_config(&self) -> RpcResult<NodeConfig> {
        self.config.clone().ok_or_else(|| {
            ErrorObjectOwned::owned(
                jsonrpsee::types::error::INTERNAL_ERROR_CODE,
                "node_getConfig is not supported on this node",
                None::<()>,
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use katana_chain_spec::ChainSpec;
    use katana_node_config::rpc::{RpcModuleKind, RpcModulesList};
    use katana_rpc_api::node::NodeApiServer;
    use katana_rpc_types::node::{
        ChainKind, DbConfigDump, DevConfigDump, ExecutionConfigDump, NodeConfig, NodeInfo,
        RpcConfigDump, SequencingConfigDump,
    };

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

    fn sample_config() -> NodeConfig {
        let mut apis = RpcModulesList::new();
        apis.add(RpcModuleKind::Node);

        NodeConfig {
            chain_id: ChainSpec::dev().id().id(),
            db: DbConfigDump { dir: None, migrate: false },
            forking: None,
            rpc: RpcConfigDump {
                addr: "127.0.0.1".parse().unwrap(),
                port: 5050,
                explorer: None,
                apis,
                cors_origins: Vec::new(),
                max_connections: None,
                max_concurrent_estimate_fee_requests: None,
                max_request_body_size: None,
                max_response_body_size: None,
                timeout_millis: None,
                max_proof_keys: None,
                max_event_page_size: None,
                max_call_gas: None,
            },
            gateway: None,
            metrics: None,
            execution: ExecutionConfigDump {
                invocation_max_steps: 10_000_000,
                validation_max_steps: 1_000_000,
                max_recursion_depth: 1000,
                compile_native: None,
            },
            messaging: None,
            sequencing: SequencingConfigDump {
                block_time: Some(5000),
                no_mining: false,
                block_cairo_steps_limit: None,
                no_state_trie: false,
            },
            dev: DevConfigDump { fee: true, account_validation: true, fixed_gas_prices: None },
            paymaster: None,
            tee: None,
            grpc: None,
        }
    }

    #[tokio::test]
    async fn get_info_returns_configured_info() {
        let info = sample_info();
        let api = NodeApi::new(info.clone(), None);
        assert_eq!(api.get_info().await.unwrap(), info);
    }

    #[tokio::test]
    async fn get_config_returns_configured_config() {
        let config = sample_config();
        let api = NodeApi::new(sample_info(), Some(config.clone()));
        assert_eq!(api.get_config().await.unwrap(), config);
    }

    #[tokio::test]
    async fn get_config_errors_when_not_configured() {
        let api = NodeApi::new(sample_info(), None);
        assert!(api.get_config().await.is_err());
    }
}
