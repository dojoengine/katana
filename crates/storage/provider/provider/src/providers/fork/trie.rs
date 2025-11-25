use std::collections::{BTreeMap, HashMap, HashSet};

use katana_db::abstraction::Database;
use katana_db::tables;
use katana_db::trie::TrieDbMut;
use katana_primitives::block::BlockNumber;
use katana_primitives::class::{ClassHash, CompiledClassHash};
use katana_primitives::state::StateUpdates;
use katana_primitives::{ContractAddress, Felt};
use katana_provider_api::state::{StateFactoryProvider, StateProvider, StateRootProvider};
use katana_provider_api::trie::TrieWriter;
use katana_provider_api::ProviderError;
use katana_rpc_types::ContractStorageKeys;
use katana_trie::{
    compute_contract_state_hash, ContractLeaf, MultiProof, PartialClassesTrie,
    PartialContractsTrie, PartialStoragesTrie,
};
use starknet::macros::short_string;

use super::ForkedProvider;
use crate::ProviderResult;

impl<Tx1: DbTxMut> TrieWriter for ForkedProvider<Tx1> {
    fn trie_insert_contract_updates(
        &self,
        block_number: BlockNumber,
        state_updates: &StateUpdates,
    ) -> ProviderResult<Felt> {
        self.local_db.trie_insert_contract_updates(block_number, state_updates)
    }

    fn trie_insert_declared_classes(
        &self,
        block_number: BlockNumber,
        updates: &BTreeMap<ClassHash, CompiledClassHash>,
    ) -> ProviderResult<Felt> {
        self.local_db.trie_insert_declared_classes(block_number, updates)
    }

    fn trie_insert_declared_classes_with_proof(
        &self,
        block_number: BlockNumber,
        updates: &BTreeMap<ClassHash, CompiledClassHash>,
        proof: MultiProof,
        original_root: Felt,
    ) -> ProviderResult<Felt> {
        self.local_db.db().update(|tx| {
            let mut trie =
                PartialClassesTrie::new_partial(TrieDbMut::<tables::ClassesTrie, _>::new(tx));

            for (class_hash, compiled_hash) in updates {
                trie.insert(*class_hash, *compiled_hash, proof.clone(), original_root);
            }

            trie.commit(block_number);
            Ok(trie.root())
        })?
    }

    fn trie_insert_contract_updates_with_proof(
        &self,
        block_number: BlockNumber,
        state_updates: &StateUpdates,
        proof: MultiProof,
        original_root: Felt,
        contract_leaves_data: HashMap<ContractAddress, ContractLeaf>,
        contracts_storage_proofs: Vec<MultiProof>,
    ) -> ProviderResult<Felt> {
        self.local_db.db().update(|tx| {
            let mut contract_trie_db =
                PartialContractsTrie::new_partial(TrieDbMut::<tables::ContractsTrie, _>::new(tx));

            let mut contract_leafs: HashMap<ContractAddress, ContractLeaf> = HashMap::new();

            // Verify that storage updates and storage proofs have matching lengths
            if state_updates.storage_updates.len() != contracts_storage_proofs.len() {
                return Err(ProviderError::ParsingError(
                    "storage updates/proofs count mismatch".to_string(),
                ));
            }

            let leaf_hashes: Vec<_> = {
                // First handle storage updates with proofs
                for ((address, storage_entries), storage_proof) in
                    state_updates.storage_updates.iter().zip(contracts_storage_proofs.iter())
                {
                    let mut storage_trie_db = PartialStoragesTrie::new_partial(
                        TrieDbMut::<tables::StoragesTrie, _>::new(tx),
                        *address,
                    );

                    // Get the original root from the contract leaf's storage_root
                    let original_storage_root = contract_leaves_data
                        .get(address)
                        .and_then(|leaf| leaf.storage_root)
                        .unwrap_or(Felt::ZERO);

                    for (key, value) in storage_entries {
                        storage_trie_db.insert(
                            *key,
                            *value,
                            storage_proof.clone(),
                            original_storage_root,
                        );
                    }

                    contract_leafs.insert(*address, Default::default());
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
                        // Use storage root from contract_leaves_data if available, otherwise get from trie
                        if leaf.storage_root.is_none() {
                            let storage_trie = PartialStoragesTrie::new_partial(
                                TrieDbMut::<tables::StoragesTrie, _>::new(tx),
                                address,
                            );
                            let storage_root = storage_trie.root();
                            // Only update storage root if we have local changes (non-zero root)
                            if storage_root != Felt::ZERO {
                                leaf.storage_root = Some(storage_root);
                            } else if let Some(leaf_data) = contract_leaves_data.get(&address) {
                                leaf.storage_root = leaf_data.storage_root;
                            }
                        }

                        // Merge with contract_leaves_data to get nonce/class_hash if not in updates
                        if let Some(leaf_data) = contract_leaves_data.get(&address) {
                            if leaf.nonce.is_none() {
                                leaf.nonce = leaf_data.nonce;
                            }
                            if leaf.class_hash.is_none() {
                                leaf.class_hash = leaf_data.class_hash;
                            }
                            if leaf.storage_root.is_none() {
                                leaf.storage_root = leaf_data.storage_root;
                            }
                        }

                        let state = if block_number == 0 {
                            StateFactoryProvider::latest(&*self.local_db)? // this will just default to an empty state
                        } else {
                            StateFactoryProvider::historical(
                                &*self.local_db,
                                (block_number - 1).into(),
                            )?
                            .expect("historical state should exist")
                        };

                        // If storage_root is still None, get it from the previous state
                        // This handles cases where contract has nonce/class changes but no storage updates
                        // and the contract wasn't in the remote proof response
                        if leaf.storage_root.is_none() {
                            if let Ok(Some(prev_storage_root)) = state.storage_root(address) {
                                leaf.storage_root = Some(prev_storage_root);
                            } else {
                                // If no previous storage root exists, use ZERO (empty storage)
                                leaf.storage_root = Some(Felt::ZERO);
                            }
                        }

                        let leaf_hash = contract_state_leaf_hash(state, &address, &leaf);

                        Ok((address, leaf_hash))
                    })
                    .collect::<Result<Vec<_>, ProviderError>>()?
            };

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
        // Collect all needed data from StateUpdates
        let mut class_hashes = Vec::new();
        let mut contract_addresses = HashSet::new();
        let mut contracts_storage_keys = Vec::new();

        // Collect class hashes
        for class_hash in state_updates.declared_classes.keys() {
            class_hashes.push(*class_hash);
        }

        // Collect all unique contract addresses that need proofs
        for address in state_updates.deployed_contracts.keys() {
            contract_addresses.insert(*address);
        }
        for address in state_updates.replaced_classes.keys() {
            contract_addresses.insert(*address);
        }
        for address in state_updates.nonce_updates.keys() {
            contract_addresses.insert(*address);
        }
        for (address, storage_map) in &state_updates.storage_updates {
            contract_addresses.insert(*address);
            let keys = storage_map.keys().cloned().collect::<Vec<_>>();
            contracts_storage_keys.push(ContractStorageKeys { address: *address, keys });
        }

        // Convert HashSet to sorted Vec for consistent ordering
        // Sorting is critical: remote RPC returns contract_leaves_data in the same order
        // as the addresses were passed, so we must ensure deterministic ordering
        let mut contract_addresses: Vec<_> = contract_addresses.into_iter().collect();
        contract_addresses.sort();

        // Fetch proofs from remote RPC
        let fork_point = self.fork_db.block_id;

        // Fetch classes proof
        let classes_proof_result = if !class_hashes.is_empty() {
            self.fork_db.backend.get_classes_proofs(class_hashes.clone(), fork_point)?
        } else {
            None
        };

        // Fetch contracts proof
        let contracts_proof_result = if !contract_addresses.is_empty() {
            self.fork_db.backend.get_contracts_proofs(contract_addresses.clone(), fork_point)?
        } else {
            None
        };

        // Fetch storages proofs
        let storages_proof_result = if !contracts_storage_keys.is_empty() {
            self.fork_db.backend.get_storages_proofs(contracts_storage_keys.clone(), fork_point)?
        } else {
            None
        };

        // Fetch global roots
        let global_roots_result = self.fork_db.backend.get_global_roots(fork_point)?;

        // Extract proofs and roots
        let (classes_proof, classes_tree_root) = if let Some(proof_response) = classes_proof_result
        {
            let proof: MultiProof = proof_response.classes_proof.nodes.into();
            let root = proof_response.global_roots.classes_tree_root;
            (Some(proof), root)
        } else {
            // No classes to update, get root from global_roots
            let root = global_roots_result
                .as_ref()
                .map(|r| r.global_roots.classes_tree_root)
                .unwrap_or(Felt::ZERO);
            (None, root)
        };

        let (contracts_proof, contracts_tree_root, contract_leaves_data) =
            if let Some(proof_response) = contracts_proof_result {
                let proof: MultiProof = proof_response.contracts_proof.nodes.into();
                let root = proof_response.global_roots.contracts_tree_root;

                // Convert contract_leaves_data to HashMap<ContractAddress, ContractLeaf>
                let leaves_map: HashMap<ContractAddress, ContractLeaf> = proof_response
                    .contracts_proof
                    .contract_leaves_data
                    .iter()
                    .zip(contract_addresses.iter())
                    .map(|(leaf_data, &addr)| {
                        let leaf = ContractLeaf {
                            storage_root: Some(leaf_data.storage_root),
                            nonce: Some(leaf_data.nonce),
                            class_hash: Some(leaf_data.class_hash),
                        };
                        (addr, leaf)
                    })
                    .collect();

                (Some(proof), root, leaves_map)
            } else {
                // No contracts to update, get root from global_roots
                let root = global_roots_result
                    .as_ref()
                    .map(|r| r.global_roots.contracts_tree_root)
                    .unwrap_or(Felt::ZERO);
                (None, root, HashMap::new())
            };

        // Convert storage proofs
        let contracts_storage_proofs: Vec<MultiProof> =
            if let Some(proof_response) = storages_proof_result {
                proof_response
                    .contracts_storage_proofs
                    .nodes
                    .into_iter()
                    .map(|nodes| nodes.into())
                    .collect()
            } else {
                Vec::new()
            };

        // Get global roots if not already fetched
        let (final_classes_root, final_contracts_root) = if let Some(roots) = global_roots_result {
            (roots.global_roots.classes_tree_root, roots.global_roots.contracts_tree_root)
        } else {
            (classes_tree_root, contracts_tree_root)
        };

        // Check if we have any local changes
        let has_class_changes = !state_updates.declared_classes.is_empty();
        //TODO: add check for deprecated classes

        let has_contract_changes = !state_updates.deployed_contracts.is_empty()
            || !state_updates.replaced_classes.is_empty()
            || !state_updates.nonce_updates.is_empty()
            || !state_updates.storage_updates.is_empty();

        // Use proof-based methods if we have changes and proofs
        let class_trie_root = if has_class_changes {
            if let Some(proof) = classes_proof {
                self.trie_insert_declared_classes_with_proof(
                    block_number,
                    &state_updates.declared_classes,
                    proof,
                    final_classes_root,
                )?
            } else {
                // Fallback to regular method if no proof
                // This will create a full trie and possibly cause problems with the state root
                // There is no conversion method for full tries to partial tries yet
                self.trie_insert_declared_classes(block_number, &state_updates.declared_classes)?
            }
        } else {
            // When no changes nothing will be fetched from mainnet and no partial trie can be constructed
            // Use the class trie root from forked network
            final_classes_root
        };

        let contract_trie_root = if has_contract_changes {
            if let Some(proof) = contracts_proof {
                self.trie_insert_contract_updates_with_proof(
                    block_number,
                    state_updates,
                    proof,
                    final_contracts_root,
                    contract_leaves_data,
                    contracts_storage_proofs,
                )?
            } else {
                // Fallback to regular method if no proof
                // This will create a full trie and possibly cause problems with the state root
                // There is no conversion method for full tries to partial tries yet
                self.trie_insert_contract_updates(block_number, state_updates)?
            }
        } else {
            // When no changes nothing will be fetched from mainnet and no partial trie can be constructed
            // Use the contract trie root from forked network
            final_contracts_root
        };

        use katana_primitives::hash::StarkHash;
        Ok(katana_primitives::hash::Poseidon::hash_array(&[
            short_string!("STARKNET_STATE_V0"),
            contract_trie_root,
            class_trie_root,
        ]))
    }
}

// computes the contract state leaf hash
fn contract_state_leaf_hash(
    provider: impl StateProvider,
    address: &ContractAddress,
    contract_leaf: &ContractLeaf,
) -> Felt {
    let nonce =
        contract_leaf.nonce.unwrap_or(provider.nonce(*address).unwrap().unwrap_or_default());

    let class_hash = contract_leaf
        .class_hash
        .unwrap_or(provider.class_hash_of_contract(*address).unwrap().unwrap_or_default());

    let storage_root = contract_leaf.storage_root.expect("root need to set");

    compute_contract_state_hash(&class_hash, &storage_root, &nonce)
}
