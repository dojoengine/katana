use super::ForkedProvider;
use crate::error::ProviderError;
use crate::providers::db::trie::contract_state_leaf_hash;
use crate::traits::state::StateFactoryProvider;
use crate::traits::trie::TrieWriter;
use crate::ProviderResult;
use jsonrpsee::core::client::ClientT;
use jsonrpsee::http_client::HttpClientBuilder;
use jsonrpsee::rpc_params;
use katana_db::abstraction::Database;
use katana_db::tables;
use katana_db::trie::TrieDbMut;
use katana_fork::StorageProofPayload;
use katana_primitives::block::{BlockIdOrTag, BlockNumber, BlockTag};
use katana_primitives::class::{ClassHash, CompiledClassHash};
use katana_primitives::hash::StarkHash;
use katana_primitives::state::StateUpdates;
use katana_primitives::{ContractAddress, Felt};
use katana_rpc_types::trie::{ContractLeafData, ContractStorageKeys, GetStorageProofResponse};
use katana_trie::bonsai::trie::trees::PartialMerkleTrees;
use katana_trie::{ClassesTrie, ContractLeaf, ContractsTrie, MultiProof, StoragesTrie};
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::sync::Arc;
use futures::executor;

impl<Db: Database + 'static> TrieWriter for ForkedProvider<Db> {
    fn trie_insert_declared_classes(
        &self,
        block_number: BlockNumber,
        updates: &BTreeMap<ClassHash, CompiledClassHash>,
    ) -> ProviderResult<Felt> {
        self.provider.trie_insert_declared_classes(block_number, updates)
    }

    fn trie_insert_declared_classes_with_proof(
        &self,
        block_number: BlockNumber,
        updates: &BTreeMap<ClassHash, CompiledClassHash>,
        proof: MultiProof,
        original_root: Felt,
    ) -> ProviderResult<Felt> {
        self.provider.0.update(|tx| {
            let mut trie = ClassesTrie::<
                _,
                PartialMerkleTrees<katana_primitives::hash::Poseidon, _, katana_trie::CommitId>,
            >::new_partial(TrieDbMut::<tables::ClassesTrie, _>::new(tx));

            for (class_hash, compiled_hash) in updates {
                trie.insert(*class_hash, *compiled_hash, proof.clone(), original_root);
            }

            trie.commit(block_number);
            Ok(trie.root())
        })?
    }

    fn trie_insert_contract_updates(
        &self,
        block_number: BlockNumber,
        state_updates: &StateUpdates,
    ) -> ProviderResult<Felt> {
        self.provider.trie_insert_contract_updates(block_number, state_updates)
    }

    fn trie_insert_contract_updates_with_proof(
        &self,
        block_number: BlockNumber,
        state_updates: &StateUpdates,
        proof: MultiProof,
        original_root: Felt,
        contract_leaves_map: HashMap<ContractAddress, ContractLeaf>,
        contracts_storage_proofs: Vec<MultiProof>,
    ) -> ProviderResult<Felt> {
        self.provider.0.update(|tx| {
            let mut contract_trie_db =
                ContractsTrie::<
                    _,
                    PartialMerkleTrees<katana_primitives::hash::Pedersen, _, katana_trie::CommitId>,
                >::new_partial(TrieDbMut::<tables::ContractsTrie, _>::new(tx));

            let mut contract_leafs: HashMap<ContractAddress, ContractLeaf> = HashMap::new();

            for (address, leaf_data) in &contract_leaves_map {
                let mut leaf = ContractLeaf::default();
                leaf.storage_root = leaf_data.storage_root;
                leaf.nonce = leaf_data.nonce;
                leaf.class_hash = leaf_data.class_hash;
                contract_leafs.insert(*address, leaf);
            }
            println!("CONTRACT LEAFS after inserting leaf from proof: {:?}", contract_leafs);

            let leaf_hashes: Vec<_> = {
                // First handle storage updates
                for ((address, storage_entries), storage_proof) in state_updates.storage_updates.iter().zip(contracts_storage_proofs.iter()) {
                    let mut storage_trie_db =
                        StoragesTrie::new_partial(TrieDbMut::<tables::StoragesTrie, _>::new(tx), *address);

                    // Get the original root from the contract leaf's storage_root
                    let original_root = contract_leaves_map.get(address)
                        .and_then(|leaf| leaf.storage_root)
                        .unwrap_or(Felt::ZERO);

                    for (key, value) in storage_entries {
                        storage_trie_db.insert(*key, *value, storage_proof.clone(), original_root);
                    }
                    contract_leafs.entry(*address).or_insert(ContractLeaf::default());
                    storage_trie_db.commit(block_number);
                }

                // Handle other contract updates
                for (address, nonce) in &state_updates.nonce_updates {
                    contract_leafs.entry(*address).or_default().nonce = Some(*nonce);
                }

                for (address, class_hash) in &state_updates.deployed_contracts {
                    contract_leafs.entry(*address).or_default().class_hash = Some(*class_hash);
                }

                for (address, class_hash) in &state_updates.replaced_classes {
                    contract_leafs.entry(*address).or_default().class_hash = Some(*class_hash);
                }

                contract_leafs
                    .into_iter()
                    .map(|(address, mut leaf)| {
                        let storage_trie = StoragesTrie::new_partial(
                            TrieDbMut::<tables::StoragesTrie, _>::new(tx),
                            address,
                        );
                        let storage_root = storage_trie.root();
                        // Only update storage root if we have local changes (non-zero root)
                        //THIS might cause a bug!
                        if storage_root != Felt::ZERO {
                            leaf.storage_root = Some(storage_root);
                        }
                        
                        let latest_state = self.latest_with_tx(tx)?;
                        let leaf_hash = contract_state_leaf_hash(latest_state, &address, &leaf);

                        Ok((address, leaf_hash))
                    })
                    .collect::<Result<Vec<_>, ProviderError>>()?
            };
            println!("LEAF HASHES FOR FORKED NETWORK: {:?}", leaf_hashes);

            for (k, v) in leaf_hashes {
                contract_trie_db.insert(k, v, proof.clone(), original_root);
            }

            contract_trie_db.commit(block_number);
            Ok(contract_trie_db.root())
        })?
    }

    fn compute_state_root(
        &self,
        block_number: BlockNumber,
        state_updates: &StateUpdates,
    ) -> ProviderResult<Felt> {
        let result: Result<(Option<GetStorageProofResponse>, Vec<ContractAddress>), ProviderError> = {
            let fork_url = self.fork_url.clone();
            let state_updates_clone = state_updates.clone();

            let url_with_port = if fork_url.port().is_none() {
                let default_port = match fork_url.scheme() {
                    "https" => ":443",
                    "http" => ":80",
                    _ => {
                        return Err(ProviderError::ParsingError(format!(
                            "Unsupported URL scheme: {}",
                            fork_url.scheme()
                        )))
                    }
                };
                format!(
                    "{}://{}{}{}",
                    fork_url.scheme(),
                    fork_url.host_str().unwrap_or(""),
                    default_port,
                    fork_url.path()
                )
            } else {
                fork_url.to_string()
            };

            let client =
                HttpClientBuilder::default().build(&url_with_port).map_err(|e| {
                    ProviderError::ParsingError(format!(
                        "Failed to create HTTP client: {}",
                        e
                    ))
                })?;
            println!("CLIENT: {:?}", client);

            // Collect storage proof data
            let mut class_hashes = Vec::new();
            let mut contract_addresses = std::collections::HashSet::new();
            let mut contracts_storage_keys = Vec::new();

            for class_hash in state_updates_clone.declared_classes.keys() {
                class_hashes.push(*class_hash);
            }

            // Collect all unique contract addresses that need proofs
            for address in state_updates_clone.deployed_contracts.keys() {
                contract_addresses.insert(*address);
            }
            for address in state_updates_clone.replaced_classes.keys() {
                contract_addresses.insert(*address);
            }
            for address in state_updates_clone.nonce_updates.keys() {
                contract_addresses.insert(*address);
            }

            for (address, storage_map) in &state_updates_clone.storage_updates {
                contract_addresses.insert(*address);
                contracts_storage_keys.push(ContractStorageKeys {
                    address: *address,
                    keys: storage_map.keys().cloned().collect(),
                });
            }

            // Convert HashSet to sorted Vec
            let mut contract_addresses: Vec<_> = contract_addresses.into_iter().collect();
            contract_addresses.sort(); //think if we need to sort the contract addresses, that may cause a bug
            let contract_addresses_clone = contract_addresses.clone();

            let response = self.backend.get_storage_proof(StorageProofPayload {
                block_number,
                class_hashes: Some(class_hashes),
                contract_addresses: Some(contract_addresses),
                contracts_storage_keys: Some(contracts_storage_keys),
                fork_url: url_with_port,
            })?;
            Ok((response, contract_addresses_clone))
        };

        println!("\nResult of starknet_getStorageProof: {:?}\n", result);
        match result {
            Ok((Some(proof), contract_addresses)) => {
                // Extract proofs from the response
                let classes_proof = MultiProof::from(proof.classes_proof.nodes.clone());
                let contracts_proof = MultiProof::from(proof.contracts_proof.nodes.clone());
                let mut contracts_storage_proofs_nodes = proof.contracts_storage_proofs.nodes.clone();
                let mut contracts_storage_proofs = Vec::new();
                for node in contracts_storage_proofs_nodes.iter_mut() {
                    let storage_proof = MultiProof::from(node.clone());
                    contracts_storage_proofs.push(storage_proof);
                }
                // let contract_leaves_data = proof.contracts_proof.contract_leaves_data.clone();
                let classes_tree_root = proof.global_roots.classes_tree_root;
                let contracts_tree_root = proof.global_roots.contracts_tree_root;
                println!("\nPROOF GLOBAL ROOTS: {:?}\n", proof.global_roots);
                println!("\nState updates: {:?}\n", state_updates);

                let contract_leaves_map: HashMap<ContractAddress, ContractLeaf> = proof.contracts_proof.contract_leaves_data
                    .iter()
                    .zip(contract_addresses.iter())
                    .map(|(leaf_data, &addr)| {
                        let mut leaf = ContractLeaf::default();
                        leaf.storage_root = Some(leaf_data.storage_root);
                        leaf.nonce = Some(leaf_data.nonce);
                        leaf.class_hash = Some(leaf_data.class_hash);
                        (addr, leaf)
                    })
                    .collect();

                // Check if we have any local changes
                let has_class_changes = !state_updates.declared_classes.is_empty()
                    || !state_updates.deprecated_declared_classes.is_empty();

                let has_contract_changes = !state_updates.deployed_contracts.is_empty()
                    || !state_updates.replaced_classes.is_empty()
                    || !state_updates.nonce_updates.is_empty()
                    || !state_updates.storage_updates.is_empty();

                let class_trie_root = if has_class_changes {
                    self.trie_insert_declared_classes_with_proof(
                        block_number,
                        &state_updates.declared_classes,
                        classes_proof,
                        classes_tree_root,
                    )?
                } else {
                    // Use the class trie root from forked network
                    classes_tree_root
                };
                println!("Class trie root: {:?}", class_trie_root);

                let contract_trie_root = if has_contract_changes {
                    self.trie_insert_contract_updates_with_proof(
                        block_number,
                        state_updates,
                        contracts_proof,
                        contracts_tree_root,
                        contract_leaves_map,
                        contracts_storage_proofs,
                    )?
                } else {
                    // Use the contract trie root from forked network
                    contracts_tree_root
                };
                println!("Contract trie root: {:?}", contract_trie_root);
                println!("STATE ROOT COMPUTED FOR FORKED NETWORK ✅: {:?}", starknet_types_core::hash::Poseidon::hash_array(&[
                    starknet::macros::short_string!("STARKNET_STATE_V0"),
                    contract_trie_root,
                    class_trie_root,
                ]));

                Ok(starknet_types_core::hash::Poseidon::hash_array(&[
                    starknet::macros::short_string!("STARKNET_STATE_V0"),
                    contract_trie_root,
                    class_trie_root,
                ]))
            }
            Ok((None, _)) => {
                tracing::error!("Failed to get storage proof for block {}", block_number);
                Err(ProviderError::ParsingError(format!("Storage proof failed")))
            }
            Err(e) => {
                tracing::error!("Failed to get storage proof for block {}: {:?}", block_number, e);
                Err(ProviderError::ParsingError(format!("Storage proof failed: {:?}", e)))
            }
        }
    }
}
