use std::path::PathBuf;

use assert_matches::assert_matches;
use jsonrpsee::core::ClientError;
use katana_chain_spec::ChainSpec;
use katana_primitives::block::BlockIdOrTag;
use katana_primitives::class::{ClassHash, CompiledClassHash};
use katana_primitives::contract::{StorageKey, StorageValue};
use katana_primitives::{hash, ContractAddress, Felt};
use katana_rpc_api::starknet::StarknetApiClient;
use katana_rpc_types::trie::ContractStorageKeys;
use katana_sequencer_node::config::rpc::DEFAULT_RPC_MAX_PROOF_KEYS;
use katana_trie::{
    compute_classes_trie_value, compute_contract_state_hash, ClassesMultiProof, MultiProof,
};
use katana_utils::TestNode;
use starknet::accounts::{Account, SingleOwnerAccount};
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;
use starknet::signers::LocalWallet;

mod common;

#[tokio::test]
async fn proofs_limit() {
    use serde_json::json;

    let sequencer = TestNode::new().await;

    // We need to use the jsonrpsee client because `starknet-rs` doesn't yet support RPC 0
    let client = sequencer.rpc_http_client();

    // Because we're using the default configuration for instantiating the node, the RPC limit is
    // set to 100. The total keys is 35 + 35 + 35 = 105.

    // Generate dummy keys
    let mut classes = Vec::new();
    let mut contracts = Vec::new();
    let mut storages = Vec::new();

    for i in 0..35 {
        storages.push(Default::default());
        classes.push(ClassHash::from(i as u64));
        contracts.push(Felt::from(i as u64).into());
    }

    let err = client
        .get_storage_proof(BlockIdOrTag::Latest, Some(classes), Some(contracts), Some(storages))
        .await
        .expect_err("rpc should enforce limit");

    assert_matches!(err, ClientError::Call(e) => {
        assert_eq!(e.code(), 1000);
        assert_eq!(&e.message(), &"Proof limit exceeded");

        let expected_data = json!({
            "total": 105,
            "limit": DEFAULT_RPC_MAX_PROOF_KEYS,
        });

        let actual_data = e.data().expect("must have data");
        let actual_data = serde_json::to_value(actual_data).unwrap();

        assert_eq!(actual_data, expected_data);
    });
}

#[tokio::test]
async fn genesis_states() {
    let sequencer = TestNode::new().await;
    let ChainSpec::Dev(chain_spec) = sequencer.backend().chain_spec.as_ref() else {
        panic!("should be dev chain spec")
    };

    let genesis_states = chain_spec.state_updates();

    // We need to use the jsonrpsee client because `starknet-rs` doesn't yet support RPC 0.8.0
    let client = sequencer.rpc_http_client();

    // Check class declarations
    let genesis_classes =
        genesis_states.state_updates.declared_classes.keys().cloned().collect::<Vec<ClassHash>>();

    // Check contract deployments
    let genesis_contracts = genesis_states
        .state_updates
        .deployed_contracts
        .keys()
        .cloned()
        .collect::<Vec<ContractAddress>>();

    // Check contract storage
    let genesis_contract_storages = genesis_states
        .state_updates
        .storage_updates
        .iter()
        .map(|(address, keys)| ContractStorageKeys {
            address: *address,
            keys: keys.keys().cloned().collect(),
        })
        .collect::<Vec<ContractStorageKeys>>();

    let proofs = client
        .get_storage_proof(
            BlockIdOrTag::Latest,
            Some(genesis_classes.clone()),
            Some(genesis_contracts.clone()),
            Some(genesis_contract_storages.clone()),
        )
        .await
        .expect("failed to get state proofs");

    // -----------------------------------------------------------------------
    // Verify classes proofs

    let classes_proof = MultiProof::from(proofs.classes_proof.nodes);
    let classes_tree_root = proofs.global_roots.classes_tree_root;
    let classes_verification_result = katana_trie::verify_proof::<hash::Poseidon>(
        &classes_proof,
        classes_tree_root,
        genesis_classes,
    );

    // Compute the classes trie values
    let class_trie_entries = genesis_states
        .state_updates
        .declared_classes
        .values()
        .map(|compiled_hash| compute_classes_trie_value(*compiled_hash))
        .collect::<Vec<Felt>>();

    assert_eq!(class_trie_entries, classes_verification_result);

    // -----------------------------------------------------------------------
    // Verify contracts proofs

    let contracts_proof = MultiProof::from(proofs.contracts_proof.nodes);
    let contracts_tree_root = proofs.global_roots.contracts_tree_root;
    let contracts_verification_result = katana_trie::verify_proof::<hash::Pedersen>(
        &contracts_proof,
        contracts_tree_root,
        genesis_contracts.into_iter().map(Felt::from).collect(),
    );

    // Compute the classes trie values
    let contracts_trie_entries = proofs
        .contracts_proof
        .contract_leaves_data
        .into_iter()
        .map(|d| compute_contract_state_hash(&d.class_hash, &d.storage_root, &d.nonce))
        .collect::<Vec<Felt>>();

    assert_eq!(contracts_trie_entries, contracts_verification_result);

    // -----------------------------------------------------------------------
    // Verify contracts proofs

    let storages_updates = &genesis_states.state_updates.storage_updates.values();
    let storages_proofs = proofs.contracts_storage_proofs.nodes;

    // The order of which the proofs are returned is of the same order of the proofs requests.
    for (storages, proofs) in storages_updates.clone().zip(storages_proofs) {
        let storage_keys = storages.keys().cloned().collect::<Vec<StorageKey>>();
        let storage_values = storages.values().cloned().collect::<Vec<StorageValue>>();

        let contracts_storages_proof = MultiProof::from(proofs);
        let (storage_tree_root, ..) = contracts_storages_proof.0.first().unwrap();

        let storages_verification_result = katana_trie::verify_proof::<hash::Pedersen>(
            &contracts_storages_proof,
            *storage_tree_root,
            storage_keys,
        );

        assert_eq!(storage_values, storages_verification_result);
    }
}

#[tokio::test]
async fn classes_proofs() {
    let sequencer = TestNode::new().await;
    let account = sequencer.account();
    let rpc_client = sequencer.starknet_rpc_client();

    let (class_hash1, compiled_class_hash1) =
        declare(&rpc_client, &account, "tests/test_data/cairo1_contract.json").await;
    let (class_hash2, compiled_class_hash2) =
        declare(&rpc_client, &account, "tests/test_data/cairo_l1_msg_contract.json").await;
    let (class_hash3, compiled_class_hash3) =
        declare(&rpc_client, &account, "tests/test_data/test_sierra_contract.json").await;

    // We need to use the jsonrpsee client because `starknet-rs` doesn't yet support RPC 0.8.0
    let client = sequencer.rpc_http_client();

    {
        let class_hash = class_hash1;
        let trie_entry = compute_classes_trie_value(compiled_class_hash1);

        let proofs = client
            .get_storage_proof(BlockIdOrTag::Number(1), Some(vec![class_hash]), None, None)
            .await
            .expect("failed to get storage proof");

        let results = ClassesMultiProof::from(MultiProof::from(proofs.classes_proof.nodes))
            .verify(proofs.global_roots.classes_tree_root, vec![class_hash]);

        assert_eq!(vec![trie_entry], results);
    }

    {
        let class_hash = class_hash2;
        let trie_entry = compute_classes_trie_value(compiled_class_hash2);

        let proofs = client
            .get_storage_proof(BlockIdOrTag::Number(2), Some(vec![class_hash]), None, None)
            .await
            .expect("failed to get storage proof");

        let results = ClassesMultiProof::from(MultiProof::from(proofs.classes_proof.nodes))
            .verify(proofs.global_roots.classes_tree_root, vec![class_hash]);

        assert_eq!(vec![trie_entry], results);
    }

    {
        let class_hash = class_hash3;
        let trie_entry = compute_classes_trie_value(compiled_class_hash3);

        let proofs = client
            .get_storage_proof(BlockIdOrTag::Number(3), Some(vec![class_hash]), None, None)
            .await
            .expect("failed to get storage proof");

        let results = ClassesMultiProof::from(MultiProof::from(proofs.classes_proof.nodes))
            .verify(proofs.global_roots.classes_tree_root, vec![class_hash]);

        assert_eq!(vec![trie_entry], results);
    }

    {
        let class_hashes = vec![class_hash1, class_hash2, class_hash3];
        let trie_entries = vec![
            compute_classes_trie_value(compiled_class_hash1),
            compute_classes_trie_value(compiled_class_hash2),
            compute_classes_trie_value(compiled_class_hash3),
        ];

        let proofs = client
            .get_storage_proof(BlockIdOrTag::Latest, Some(class_hashes.clone()), None, None)
            .await
            .expect("failed to get storage proof");

        let results = ClassesMultiProof::from(MultiProof::from(proofs.classes_proof.nodes))
            .verify(proofs.global_roots.classes_tree_root, class_hashes.clone());

        assert_eq!(trie_entries, results);
    }
}

/// Test that storage proofs are returned in the same order as the request,
/// even when the request order differs from the natural sorted order.
///
/// This is critical for the forking code which relies on zip() between
/// storage_updates (BTreeMap iteration) and the RPC response.
#[tokio::test]
async fn storage_proofs_ordering_with_reversed_request() {
    let sequencer = TestNode::new().await;
    let ChainSpec::Dev(chain_spec) = sequencer.backend().chain_spec.as_ref() else {
        panic!("should be dev chain spec")
    };

    let genesis_states = chain_spec.state_updates();
    let client = sequencer.rpc_http_client();

    // Collect contracts with storage updates in sorted order (BTreeMap)
    let sorted_storage_keys: Vec<ContractStorageKeys> = genesis_states
        .state_updates
        .storage_updates
        .iter()
        .map(|(address, keys)| ContractStorageKeys {
            address: *address,
            keys: keys.keys().cloned().collect(),
        })
        .collect();

    // Need at least 2 contracts to meaningfully test ordering
    assert!(sorted_storage_keys.len() >= 2, "genesis must have at least 2 contracts with storage");

    // Create reversed order request
    let reversed_storage_keys: Vec<ContractStorageKeys> =
        sorted_storage_keys.iter().rev().cloned().collect();

    // Verify the order is actually different (sanity check)
    assert_ne!(
        sorted_storage_keys.first().unwrap().address,
        reversed_storage_keys.first().unwrap().address,
        "reversed order should differ from sorted"
    );

    // Also request contract addresses in reversed order to get storage roots via
    // contract_leaves_data
    let reversed_contracts: Vec<ContractAddress> =
        reversed_storage_keys.iter().map(|k| k.address).collect();

    let proofs = client
        .get_storage_proof(
            BlockIdOrTag::Latest,
            None,
            Some(reversed_contracts.clone()),
            Some(reversed_storage_keys.clone()),
        )
        .await
        .expect("failed to get storage proofs");

    assert_eq!(
        proofs.contracts_storage_proofs.nodes.len(),
        reversed_storage_keys.len(),
        "should have one proof per requested contract"
    );

    // Verify each proof matches the contract at that position in the reversed request.
    // If the RPC returned proofs in sorted order instead of request order, the proofs
    // would be for different contracts and verification would fail.
    for (i, (req, proof_nodes)) in
        reversed_storage_keys.iter().zip(proofs.contracts_storage_proofs.nodes.iter()).enumerate()
    {
        let expected_entries = genesis_states
            .state_updates
            .storage_updates
            .get(&req.address)
            .expect("contract should be in genesis storage updates");

        let storage_keys: Vec<StorageKey> = expected_entries.keys().cloned().collect();
        let expected_values: Vec<StorageValue> = expected_entries.values().cloned().collect();

        let proof = MultiProof::from(proof_nodes.clone());

        // Use storage root from contract_leaves_data (ordered same as reversed_contracts)
        let storage_root = proofs.contracts_proof.contract_leaves_data[i].storage_root;

        let verified_values =
            katana_trie::verify_proof::<hash::Pedersen>(&proof, storage_root, storage_keys);

        assert_eq!(
            expected_values, verified_values,
            "storage proof at position {i} should match contract {} (reversed order)",
            req.address
        );
    }
}

/// Test that contract_leaves_data is returned in the same order as the requested
/// contract addresses, even when the request order differs from sorted order.
#[tokio::test]
async fn contract_leaves_data_ordering_with_reversed_request() {
    let sequencer = TestNode::new().await;
    let ChainSpec::Dev(chain_spec) = sequencer.backend().chain_spec.as_ref() else {
        panic!("should be dev chain spec")
    };

    let genesis_states = chain_spec.state_updates();
    let client = sequencer.rpc_http_client();

    // Get genesis contract addresses in sorted order (BTreeMap)
    let sorted_contracts: Vec<ContractAddress> =
        genesis_states.state_updates.deployed_contracts.keys().cloned().collect();

    assert!(sorted_contracts.len() >= 2, "genesis must have at least 2 deployed contracts");

    // Request in sorted order
    let sorted_proofs = client
        .get_storage_proof(BlockIdOrTag::Latest, None, Some(sorted_contracts.clone()), None)
        .await
        .expect("failed to get sorted proofs");

    // Request in reversed order
    let reversed_contracts: Vec<ContractAddress> = sorted_contracts.iter().rev().cloned().collect();
    let reversed_proofs = client
        .get_storage_proof(BlockIdOrTag::Latest, None, Some(reversed_contracts.clone()), None)
        .await
        .expect("failed to get reversed proofs");

    assert_eq!(
        sorted_proofs.contracts_proof.contract_leaves_data.len(),
        reversed_proofs.contracts_proof.contract_leaves_data.len()
    );

    let n = sorted_contracts.len();

    // The reversed response's leaf data should be the reverse of the sorted response's
    for i in 0..n {
        let sorted_leaf = &sorted_proofs.contracts_proof.contract_leaves_data[i];
        let reversed_leaf = &reversed_proofs.contracts_proof.contract_leaves_data[n - 1 - i];

        assert_eq!(
            sorted_leaf.nonce,
            reversed_leaf.nonce,
            "nonce mismatch: sorted[{i}] vs reversed[{}]",
            n - 1 - i
        );
        assert_eq!(
            sorted_leaf.class_hash,
            reversed_leaf.class_hash,
            "class_hash mismatch: sorted[{i}] vs reversed[{}]",
            n - 1 - i
        );
        assert_eq!(
            sorted_leaf.storage_root,
            reversed_leaf.storage_root,
            "storage_root mismatch: sorted[{i}] vs reversed[{}]",
            n - 1 - i
        );
    }

    // Verify each leaf in the reversed response computes the correct contract state hash
    let contracts_proof = MultiProof::from(reversed_proofs.contracts_proof.nodes);
    let verified_hashes = katana_trie::verify_proof::<hash::Pedersen>(
        &contracts_proof,
        reversed_proofs.global_roots.contracts_tree_root,
        reversed_contracts.iter().map(|a| Felt::from(*a)).collect(),
    );

    for (i, (leaf_data, verified_hash)) in
        reversed_proofs.contracts_proof.contract_leaves_data.iter().zip(verified_hashes).enumerate()
    {
        let computed_hash = compute_contract_state_hash(
            &leaf_data.class_hash,
            &leaf_data.storage_root,
            &leaf_data.nonce,
        );
        assert_eq!(
            computed_hash, verified_hash,
            "contract leaf hash mismatch at position {i} for contract {}",
            reversed_contracts[i]
        );
    }
}

async fn declare(
    client: &katana_starknet::rpc::Client,
    account: &SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>,
    path: impl Into<PathBuf>,
) -> (ClassHash, CompiledClassHash) {
    let (contract, compiled_class_hash) = common::prepare_contract_declaration_params(&path.into())
        .expect("failed to prepare class declaration params");

    let class_hash = contract.class_hash();
    let res = account
        .declare_v3(contract.into(), compiled_class_hash)
        .send()
        .await
        .expect("failed to send declare tx");

    katana_utils::TxWaiter::new(res.transaction_hash, client).await.expect("failed to wait on tx");

    (class_hash, compiled_class_hash)
}
