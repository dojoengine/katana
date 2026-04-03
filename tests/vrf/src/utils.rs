use std::sync::Arc;

use axum::response::IntoResponse;
use axum::routing::post;
use katana_primitives::{ContractAddress, Felt};
use katana_rpc_types::RpcSierraContractClass;
use katana_utils::node::TestNode;
use katana_utils::TxWaiter;
use starknet::accounts::Account;
use starknet::contract::{ContractFactory, UdcSelector};
use starknet::core::utils::get_contract_address;
use tokio::net::TcpListener;
use url::Url;

katana_contracts::contract!(
    SimpleVrfAppContract,
    "{CARGO_MANIFEST_DIR}/build/vrng_test_Simple.contract_class.json"
);

/// Declares and deploys the Simple contract with the VRF provider address as constructor arg.
pub async fn bootstrap_app(node: &TestNode, vrf_provider: ContractAddress) -> Felt {
    let account = node.account();
    let provider = node.starknet_rpc_client();

    // Declare

    let sierra_class = SimpleVrfAppContract::CLASS.clone().to_sierra().unwrap();
    let rpc_sierra_class = RpcSierraContractClass::from(sierra_class);

    let class_hash = SimpleVrfAppContract::HASH;
    let casm_hash = SimpleVrfAppContract::CASM_HASH;

    let res = account
        .declare_v3(Arc::new(rpc_sierra_class.try_into().unwrap()), casm_hash)
        .send()
        .await
        .expect("declare failed");

    TxWaiter::new(res.transaction_hash, &provider).await.expect("declare tx failed");

    // Deploy with VRF provider address as constructor arg
    let salt = Felt::ZERO;
    let ctor_calldata = vec![vrf_provider.into()];

    let factory = ContractFactory::new_with_udc(class_hash, &account, UdcSelector::Legacy);
    let res = factory.deploy_v3(ctor_calldata.clone(), salt, false).send().await.unwrap();

    let address = get_contract_address(salt, class_hash, &ctor_calldata, Felt::ZERO);

    TxWaiter::new(res.transaction_hash, &provider).await.expect("deploy tx failed");

    address
}

/// Starts a minimal mock Cartridge Controller API that always returns "Address not found".
pub async fn start_mock_cartridge_api() -> Url {
    async fn handler(axum::Json(_body): axum::Json<serde_json::Value>) -> axum::response::Response {
        "Address not found".into_response()
    }

    let app = axum::Router::new().route("/accounts/calldata", post(handler));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    Url::parse(&format!("http://{addr}")).unwrap()
}
