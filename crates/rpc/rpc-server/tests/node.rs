use katana_node_config::build_info::BuildInfo;
use katana_node_config::rpc::{RpcModuleKind, RpcModulesList};
use katana_rpc_server::api::node::NodeApiClient;
use katana_rpc_types::node::ChainKind;
use katana_utils::node::test_config;
use katana_utils::TestNode;

mod common;

#[tokio::test]
async fn node_get_info_returns_chain_id_and_kind() {
    let sequencer = TestNode::new().await;
    let backend = sequencer.backend();
    let client = sequencer.rpc_http_client();

    let info = client.get_info().await.unwrap();

    assert_eq!(info.chain_id, backend.chain_spec.id().id());
    assert_eq!(info.chain_kind, ChainKind::Sequencer);
    assert!(info.dev, "TestNode uses ChainSpec::Dev, so dev flag must be true");

    // TestNode does not go through `bin/katana`, so BuildInfo retains its
    // `"unknown"` sentinels. This is expected and documents that behavior.
    assert_eq!(info.version, "unknown");
    assert_eq!(info.git_sha, "unknown");
    assert_eq!(info.build_timestamp, "unknown");
    assert!(info.features.is_empty());
}

#[tokio::test]
async fn node_get_info_surfaces_custom_build_info() {
    let mut config = test_config();
    config.build_info = BuildInfo {
        version: "1.2.3-test".into(),
        git_sha: "abcdef7".into(),
        build_timestamp: "2026-04-21T00:00:00Z".into(),
        features: vec!["native".into(), "tee".into()],
    };

    let sequencer = TestNode::new_with_config(config).await;
    let client = sequencer.rpc_http_client();

    let info = client.get_info().await.unwrap();

    assert_eq!(info.version, "1.2.3-test");
    assert_eq!(info.git_sha, "abcdef7");
    assert_eq!(info.build_timestamp, "2026-04-21T00:00:00Z");
    assert_eq!(info.features, vec!["native".to_string(), "tee".to_string()]);
}

#[tokio::test]
async fn node_api_not_registered_when_module_disabled() {
    let mut config = test_config();
    // Explicitly exclude the Node module from the API set.
    let mut apis = RpcModulesList::new();
    apis.add(RpcModuleKind::Starknet);
    config.rpc.apis = apis;

    let sequencer = TestNode::new_with_config(config).await;
    let client = sequencer.rpc_http_client();

    let result = client.get_info().await;
    assert!(result.is_err(), "expected MethodNotFound when RpcModuleKind::Node is disabled");
    // Confirm it's a MethodNotFound, not some other error
    let err_str = format!("{:?}", result.unwrap_err());
    assert!(
        err_str.contains("MethodNotFound") || err_str.contains("-32601"),
        "expected MethodNotFound error, got: {err_str}"
    );
}
