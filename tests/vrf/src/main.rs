use std::sync::Arc;

use cairo_lang_starknet_classes::casm_contract_class::CasmContractClass;
use cairo_lang_starknet_classes::contract_class::ContractClass;
use katana_primitives::block::BlockIdOrTag;
use katana_primitives::class::CompiledClass;
use katana_primitives::Felt;
use katana_utils::node::TestNode;
use starknet::accounts::Account;
use starknet::contract::{ContractFactory, UdcSelector};
use starknet::core::types::contract::SierraClass;
use starknet::core::utils::get_contract_address;

const SIERRA_CLASS: &str = include_str!("../build/vrng_test_Simple.contract_class.json");

#[tokio::main]
async fn main() {
    let node = TestNode::new().await;

    let (class_hash, contract_address) = declare_and_deploy(&node).await;

    // --- Temporary asserts to verify the contracts are declared and deployed ---

    let provider = node.starknet_rpc_client();

    let class = provider.get_class(BlockIdOrTag::PreConfirmed, class_hash).await;
    assert!(class.is_ok(), "class not found: expected {class_hash:#x} to be declared");

    let deployed_class_hash =
        provider.get_class_hash_at(BlockIdOrTag::PreConfirmed, contract_address.into()).await;
    assert!(
        deployed_class_hash.is_ok(),
        "contract not found: expected deployment at {contract_address:#x}"
    );
    assert_eq!(
        deployed_class_hash.unwrap(),
        class_hash,
        "deployed contract class hash mismatch"
    );

    println!("All assertions passed.");
}

/// Declares and deploys the VRF test contract on the given node.
///
/// Returns `(class_hash, contract_address)`.
async fn declare_and_deploy(node: &TestNode) -> (Felt, Felt) {
    let account = node.account();
    let provider = node.starknet_rpc_client();

    // --- Declare ---

    let sierra: SierraClass = serde_json::from_str(SIERRA_CLASS).expect("invalid sierra class");
    let class_hash = sierra.class_hash().expect("failed to compute class hash");
    let flattened = sierra.flatten().expect("failed to flatten sierra class");

    let contract: ContractClass =
        serde_json::from_str(SIERRA_CLASS).expect("invalid contract class");
    let casm = CasmContractClass::from_contract_class(contract, true, usize::MAX)
        .expect("failed to compile to CASM");
    let casm_json = serde_json::to_string(&casm).expect("failed to serialize CASM");
    let compiled: CompiledClass =
        serde_json::from_str(&casm_json).expect("invalid compiled class");
    let compiled_class_hash = compiled.class_hash().expect("failed to compute compiled class hash");

    let res = account
        .declare_v3(Arc::new(flattened), compiled_class_hash)
        .send()
        .await
        .expect("declare failed");

    katana_utils::TxWaiter::new(res.transaction_hash, &provider)
        .await
        .expect("declare tx failed");

    println!("Class declared: {class_hash:#x}");

    // --- Deploy ---

    let salt = Felt::ZERO;
    let constructor_calldata = vec![];
    let factory = ContractFactory::new_with_udc(class_hash, &account, UdcSelector::Legacy);
    let deployment = factory.deploy_v3(constructor_calldata.clone(), salt, false);

    let address = get_contract_address(salt, class_hash, &constructor_calldata, Felt::ZERO);

    let res = deployment.send().await.expect("deploy failed");

    katana_utils::TxWaiter::new(res.transaction_hash, &provider)
        .await
        .expect("deploy tx failed");

    println!("Contract deployed: {address:#x}");

    (class_hash, address)
}
