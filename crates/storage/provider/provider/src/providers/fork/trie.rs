use super::ForkedProvider;
use crate::error::ProviderError;
use crate::providers::db::trie::contract_state_leaf_hash;
use crate::traits::trie::TrieWriter;
use crate::ProviderResult;
use katana_db::abstraction::Database;
use katana_db::tables;
use katana_db::trie::TrieDbMut;
use katana_primitives::block::BlockNumber;
use katana_primitives::class::{ClassHash, CompiledClassHash};
use katana_primitives::state::StateUpdates;
use katana_primitives::{ContractAddress, Felt};
use katana_primitives::hash::StarkHash;
use katana_trie::bonsai::trie::trees::PartialMerkleTrees;
use katana_trie::{
    ClassesTrie, ContractLeaf, ContractsTrie, StoragesTrie, MultiProof,
};
use std::collections::{BTreeMap, HashMap};
use crate::traits::state::StateFactoryProvider;

impl<Db: Database> TrieWriter for ForkedProvider<Db> {
    fn trie_insert_declared_classes(
        &self,
        block_number: BlockNumber,
        updates: &BTreeMap<ClassHash, CompiledClassHash>,
    ) -> ProviderResult<Felt> {
        self.provider.0.update(|tx| {
            let mut trie = ClassesTrie::new(TrieDbMut::<tables::ClassesTrie, _>::new(tx));

            for (class_hash, compiled_hash) in updates {
                trie.insert(*class_hash, *compiled_hash);
            }

            trie.commit(block_number);
            Ok(trie.root())
        })?
    }

    fn trie_insert_declared_classes_with_proof(
        &self,
        block_number: BlockNumber,
        updates: &BTreeMap<ClassHash, CompiledClassHash>,
        proof: MultiProof,
        original_root: Felt,
    ) -> ProviderResult<Felt> {
        self.provider.0.update(|tx| {
            let mut trie = ClassesTrie::<_, PartialMerkleTrees<katana_primitives::hash::Poseidon, _, katana_trie::CommitId>>::new_partial(TrieDbMut::<tables::ClassesTrie, _>::new(tx));

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
        self.provider.0.update(|tx| {
            let mut contract_trie_db =
                ContractsTrie::new(TrieDbMut::<tables::ContractsTrie, _>::new(tx));

            let mut contract_leafs: HashMap<ContractAddress, ContractLeaf> = HashMap::new();

            let leaf_hashes: Vec<_> = {
                // First we insert the contract storage changes
                for (address, storage_entries) in &state_updates.storage_updates {
                    let mut storage_trie_db =
                        StoragesTrie::new(TrieDbMut::<tables::StoragesTrie, _>::new(tx), *address);

                    for (key, value) in storage_entries {
                        storage_trie_db.insert(*key, *value);
                    }
                    // insert the contract address in the contract_leafs to put the storage root
                    // later
                    contract_leafs.insert(*address, Default::default());

                    // Then we commit them
                    storage_trie_db.commit(block_number);
                }

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
                        let storage_trie = StoragesTrie::new(
                            TrieDbMut::<tables::StoragesTrie, _>::new(tx),
                            address,
                        );
                        let storage_root = storage_trie.root();
                        leaf.storage_root = Some(storage_root);

                        let latest_state = self.provider.latest()?;
                        let leaf_hash = contract_state_leaf_hash(latest_state, &address, &leaf);

                        Ok((address, leaf_hash))
                    })
                    .collect::<Result<Vec<_>, ProviderError>>()?
            };

            for (k, v) in leaf_hashes {
                contract_trie_db.insert(k, v);
            }

            contract_trie_db.commit(block_number);
            Ok(contract_trie_db.root())
        })?
    }

    fn trie_insert_contract_updates_with_proof(
        &self,
        block_number: BlockNumber,
        state_updates: &StateUpdates,
        proof: MultiProof,
        original_root: Felt,
    ) -> ProviderResult<Felt> {
        self.provider.0.update(|tx| {
            let mut contract_trie_db =
                ContractsTrie::<_, PartialMerkleTrees<katana_primitives::hash::Pedersen, _, katana_trie::CommitId>>::new_partial(TrieDbMut::<tables::ContractsTrie, _>::new(tx));

            let mut contract_leafs: HashMap<ContractAddress, ContractLeaf> = HashMap::new();

            let leaf_hashes: Vec<_> = {
                // First we insert the contract storage changes
                for (address, storage_entries) in &state_updates.storage_updates {
                    let mut storage_trie_db =
                        StoragesTrie::new(TrieDbMut::<tables::StoragesTrie, _>::new(tx), *address);

                    for (key, value) in storage_entries {
                        storage_trie_db.insert(*key, *value);
                    }
                    // insert the contract address in the contract_leafs to put the storage root
                    // later
                    contract_leafs.insert(*address, Default::default());

                    // Then we commit them
                    storage_trie_db.commit(block_number);
                }

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
                        let storage_trie = StoragesTrie::new(
                            TrieDbMut::<tables::StoragesTrie, _>::new(tx),
                            address,
                        );
                        let storage_root = storage_trie.root();
                        leaf.storage_root = Some(storage_root);

                        let latest_state = self.provider.latest()?;
                        let leaf_hash = contract_state_leaf_hash(latest_state, &address, &leaf);

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

    /// Override compute_state_root to use proof-based methods when available
    fn compute_state_root(
        &self,
        block_number: BlockNumber,
        state_updates: &StateUpdates,
    ) -> ProviderResult<Felt> {
        // TODO: Here we should check if we have a proof
        // For now we use fallback to standard methods
        if let Some((proof, original_root)) = self.get_fork_proof(block_number)? {
            let class_trie_root = self.trie_insert_declared_classes_with_proof(
                block_number,
                &state_updates.declared_classes,
                proof.clone(),
                original_root,
            )?;

            let contract_trie_root = self.trie_insert_contract_updates_with_proof(
                block_number,
                state_updates,
                proof,
                original_root,
            )?;

            Ok(starknet_types_core::hash::Poseidon::hash_array(&[
                starknet::macros::short_string!("STARKNET_STATE_V0"),
                contract_trie_root,
                class_trie_root,
            ]))
        } else {
            // Fallback to default implementation
            let class_trie_root = self.trie_insert_declared_classes(block_number, &state_updates.declared_classes)?;
            let contract_trie_root = self.trie_insert_contract_updates(block_number, state_updates)?;
            
            Ok(starknet_types_core::hash::Poseidon::hash_array(&[
                starknet::macros::short_string!("STARKNET_STATE_V0"),
                contract_trie_root,
                class_trie_root,
            ]))
        }
    }
}

impl<Db: Database> ForkedProvider<Db> {
    /// Get the fork proof for trie operations
    fn get_fork_proof(&self, _block_number: BlockNumber) -> ProviderResult<Option<(MultiProof, Felt)>> {
        // TODO: Here we should implement fetching proof from forked network
        // For now we return None, which will cause fallback to standard methods
        Ok(None)
    }
}
