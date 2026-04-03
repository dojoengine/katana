use std::sync::Arc;

use axum::response::IntoResponse;
use axum::routing::post;
use katana_primitives::{felt, ContractAddress, Felt};
use katana_rpc_types::{OutsideExecutionV2, RpcSierraContractClass};
use katana_utils::node::TestNode;
use katana_utils::TxWaiter;
use starknet::accounts::Account;
use starknet::contract::{ContractFactory, UdcSelector};
use starknet::core::utils::get_contract_address;
use starknet::signers::LocalWallet;
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

/// Signs an OutsideExecutionV2 using SNIP-12 (same hash computation as the OZ SRC9 contract).
pub async fn sign_outside_execution_v2(
    outside_execution: &OutsideExecutionV2,
    chain_id: Felt,
    signer_address: ContractAddress,
    signer: &LocalWallet,
) -> Vec<Felt> {
    use starknet::signers::Signer;
    use starknet_crypto::{poseidon_hash_many, PoseidonHasher};

    const STARKNET_DOMAIN_TYPE_HASH: Felt =
        felt!("0x1ff2f602e42168014d405a94f75e8a93d640751d71d16311266e140d8b0a210");
    const OUTSIDE_EXECUTION_TYPE_HASH: Felt =
        felt!("0x312b56c05a7965066ddbda31c016d8d05afc305071c0ca3cdc2192c3c2f1f0f");
    const CALL_TYPE_HASH: Felt =
        felt!("0x3635c7f2a7ba93844c0d064e18e487f35ab90f7c39d00f186a781fc3f0c2ca9");

    // Domain hash
    let domain_hash = poseidon_hash_many(&[
        STARKNET_DOMAIN_TYPE_HASH,
        Felt::from_bytes_be_slice(b"Account.execute_from_outside"),
        Felt::TWO,
        chain_id,
        Felt::ONE,
    ]);

    // Hash each call
    let mut hashed_calls = Vec::new();
    for call in &outside_execution.calls {
        let mut h = PoseidonHasher::new();
        h.update(CALL_TYPE_HASH);
        h.update(call.contract_address.into());
        h.update(call.entry_point_selector);
        h.update(poseidon_hash_many(&call.calldata));
        hashed_calls.push(h.finalize());
    }

    // Outside execution hash
    let mut h = PoseidonHasher::new();
    h.update(OUTSIDE_EXECUTION_TYPE_HASH);
    h.update(outside_execution.caller.into());
    h.update(outside_execution.nonce);
    h.update(Felt::from(outside_execution.execute_after));
    h.update(Felt::from(outside_execution.execute_before));
    h.update(poseidon_hash_many(&hashed_calls));
    let outside_execution_hash = h.finalize();

    // Final message hash
    let mut h = PoseidonHasher::new();
    h.update(Felt::from_bytes_be_slice(b"StarkNet Message"));
    h.update(domain_hash);
    h.update(signer_address.into());
    h.update(outside_execution_hash);
    let message_hash = h.finalize();

    let signature = signer.sign_hash(&message_hash).await.unwrap();
    vec![signature.r, signature.s]
}

pub fn find_in_path(binary: &str) -> Option<std::path::PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var)
        .map(|dir| dir.join(binary))
        .find(|candidate| candidate.is_file())
}
