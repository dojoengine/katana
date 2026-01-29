use std::collections::{HashMap, HashSet};

use katana_db::abstraction::DbTxMut;
use katana_db::tables;
use katana_db::trie::TrieDbMut;
use katana_primitives::block::BlockNumber;
use katana_primitives::class::{ClassHash, CompiledClassHash};
use katana_primitives::hash::StarkHash;
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
        let mut contracts = HashSet::new();
        let mut storage_keys = Vec::new();

        // Collect all unique contract addresses that need proofs
        for address in state_updates.deployed_contracts.keys() {
            contracts.insert(*address);
        }
        for address in state_updates.replaced_classes.keys() {
            contracts.insert(*address);
        }
        for address in state_updates.nonce_updates.keys() {
            contracts.insert(*address);
        }
        for (address, storage_map) in &state_updates.storage_updates {
            contracts.insert(*address);
            let keys = storage_map.keys().cloned().collect::<Vec<_>>();
            storage_keys.push(ContractStorageKeys { address: *address, keys });
        }

        let mut contracts: Vec<_> = contracts.into_iter().collect();
        contracts.sort();

        // Fetch contracts proof
        let contracts_proof_result = if !contracts.is_empty() {
            self.fork_db.backend.get_contracts_proofs(contracts.clone(), self.fork_db.block_id)?
        } else {
            None
        };

        // Fetch storages proofs
        let storages_proof_result = if !storage_keys.is_empty() {
            self.fork_db.backend.get_storages_proofs(storage_keys.clone(), self.fork_db.block_id)?
        } else {
            None
        };

        // Fetch global roots (always needed as fallback when no changes)
        let original_root = self
            .fork_db
            .backend
            .get_global_roots(self.fork_db.block_id)?
            .map(|roots| roots.global_roots.contracts_tree_root)
            .expect("global roots should exist for fork point");

        let (contracts_proof, contract_leaves_data) =
            if let Some(proof_response) = contracts_proof_result {
                let proof: MultiProof = proof_response.contracts_proof.nodes.into();

                // Convert contract_leaves_data to HashMap<ContractAddress, ContractLeaf>
                let leaves_map: HashMap<ContractAddress, ContractLeaf> = proof_response
                    .contracts_proof
                    .contract_leaves_data
                    .iter()
                    .zip(contracts.iter())
                    .map(|(leaf_data, &addr)| {
                        let leaf = ContractLeaf {
                            storage_root: Some(leaf_data.storage_root),
                            nonce: Some(leaf_data.nonce),
                            class_hash: Some(leaf_data.class_hash),
                        };
                        (addr, leaf)
                    })
                    .collect();

                (Some(proof), leaves_map)
            } else {
                (None, HashMap::new())
            };

        // Convert storage proofs
        let contracts_storage_proofs: Vec<MultiProof> = storages_proof_result
            .map(|response| {
                response
                    .contracts_storage_proofs
                    .nodes
                    .into_iter()
                    .map(|nodes| nodes.into())
                    .collect()
            })
            .unwrap_or_default();

        if let Some(proof) = contracts_proof {
            let mut contract_trie_db =
                PartialContractsTrie::new_partial(TrieDbMut::<tables::ContractsTrie, _>::new(
                    self.local_db.tx().clone(),
                ));

            let mut contract_leafs: HashMap<ContractAddress, ContractLeaf> = HashMap::new();

            // Verify that storage updates and storage proofs have matching lengths
            if state_updates.storage_updates.len() != contracts_storage_proofs.len() {
                return Err(ProviderError::ParsingError(
                    "storage updates/proofs count mismatch".to_string(),
                ));
            }

            let latest_state = self.latest()?;

            let leaf_hashes: Vec<_> = {
                // First handle storage updates with proofs
                for ((address, storage_entries), storage_proof) in
                    state_updates.storage_updates.iter().zip(contracts_storage_proofs.iter())
                {
                    let mut storage_trie_db = PartialStoragesTrie::new_partial(
                        TrieDbMut::<tables::StoragesTrie, _>::new(self.local_db.tx().clone()),
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
                        // Use storage root from contract_leaves_data if available, otherwise get
                        // from trie
                        if leaf.storage_root.is_none() {
                            let storage_trie = PartialStoragesTrie::new_partial(
                                TrieDbMut::<tables::StoragesTrie, _>::new(
                                    self.local_db.tx().clone(),
                                ),
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

                        // If storage_root is still None, get it from the previous state
                        // This handles cases where contract has nonce/class changes but no storage
                        // updates and the contract wasn't in the remote proof
                        // response
                        if leaf.storage_root.is_none() {
                            if let Ok(Some(prev_storage_root)) = latest_state.storage_root(address)
                            {
                                leaf.storage_root = Some(prev_storage_root);
                            } else {
                                // If no previous storage root exists, use ZERO (empty storage)
                                leaf.storage_root = Some(Felt::ZERO);
                            }
                        }

                        let leaf_hash =
                            contract_state_leaf_hash(latest_state.as_ref(), &address, &leaf);

                        Ok((address, leaf_hash))
                    })
                    .collect::<Result<Vec<_>, ProviderError>>()?
            };

            for (k, v) in leaf_hashes {
                contract_trie_db.insert(k, v, proof.clone(), original_root);
            }

            contract_trie_db.commit(block_number);
            Ok(contract_trie_db.root())
        } else {
            // No contract changes - use the fork point root (same as state.rs logic)
            Ok(original_root)
        }
    }

    fn trie_insert_declared_classes(
        &self,
        block_number: BlockNumber,
        classes: Vec<(ClassHash, CompiledClassHash)>,
    ) -> ProviderResult<Felt> {
        let class_hashes = classes.iter().map(|e| e.0).collect::<Vec<ClassHash>>();

        // Fetch proofs from remote RPC (only if we have changes)
        let classes_proof_result = if !class_hashes.is_empty() {
            self.fork_db.backend.get_classes_proofs(class_hashes, self.fork_db.block_id)?
        } else {
            None
        };

        // Extract proofs (only if we have changes)
        let classes_proof: Option<MultiProof> =
            classes_proof_result.map(|response| response.classes_proof.nodes.into());

        // Fetch global roots (always needed as fallback when no changes)
        let global_roots = self
            .fork_db
            .backend
            .get_global_roots(self.fork_db.block_id)?
            .expect("global roots should exist for fork point");

        let original_root = global_roots.global_roots.classes_tree_root;

        // Use proof-based methods if we have proofs (which means we have changes)
        // If no proofs, use the fork point root
        if let Some(proof) = classes_proof {
            let mut trie = PartialClassesTrie::new_partial(
                TrieDbMut::<tables::ClassesTrie, _>::new(self.local_db.tx().clone()),
            );

            for (class_hash, compiled_hash) in classes {
                trie.insert(class_hash, compiled_hash, proof.clone(), original_root);
            }

            trie.commit(block_number);
            Ok(trie.root())
        } else {
            Ok(original_root)
        }
    }

    fn compute_state_root(
        &self,
        block_number: BlockNumber,
        state_updates: &StateUpdates,
    ) -> ProviderResult<Felt> {
        let mut updated_classes = Vec::new();
        updated_classes.extend(state_updates.declared_classes.clone());
        updated_classes.extend(state_updates.migrated_compiled_classes.clone());

        let class_trie_root = self.trie_insert_declared_classes(block_number, updated_classes)?;
        let contract_trie_root = self.trie_insert_contract_updates(block_number, state_updates)?;

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

    let class_hash = contract_leaf.class_hash.unwrap_or_else(|| {
        provider.class_hash_of_contract(*address).ok().flatten().unwrap_or_default()
    });

    let storage_root = contract_leaf.storage_root.expect("root need to set");

    compute_contract_state_hash(&class_hash, &storage_root, &nonce)
}
