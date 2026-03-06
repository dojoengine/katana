use std::collections::HashMap;

use katana_db::abstraction::DbTxMut;
use katana_db::models::trie::TrieType;
use katana_db::tables;
use katana_db::trie::{
    next_node_index, persist_storage_trie_update, persist_trie_update, TrieDbFactory,
};
use katana_primitives::block::BlockNumber;
use katana_primitives::class::{ClassHash, CompiledClassHash};
use katana_primitives::state::StateUpdates;
use katana_primitives::{ContractAddress, Felt};
use katana_provider_api::state::{StateFactoryProvider, StateProvider};
use katana_provider_api::trie::TrieWriter;
use katana_provider_api::ProviderError;
use katana_trie::{compute_contract_state_hash, ContractLeaf};

use crate::providers::db::DbProvider;
use crate::ProviderResult;

impl<Tx: DbTxMut> TrieWriter for DbProvider<Tx> {
    fn trie_insert_declared_classes(
        &self,
        block_number: BlockNumber,
        updates: impl Iterator<Item = (ClassHash, CompiledClassHash)>,
    ) -> ProviderResult<Felt> {
        let factory = TrieDbFactory::new(self.0.clone());

        // Get previous block's root, or start with empty trie
        let prev_block = if block_number > 0 { block_number - 1 } else { block_number };
        let mut trie = factory.classes_trie(prev_block);

        for (class_hash, compiled_hash) in updates {
            trie.insert(class_hash, compiled_hash)
                .map_err(|e| ProviderError::Other(e.to_string()))?;
        }

        let update = trie.commit().map_err(|e| ProviderError::Other(e.to_string()))?;

        let root_commitment = update.root_commitment;

        // Persist the update
        let mut next_idx = next_node_index::<tables::TrieClassNodes, _>(&self.0)
            .map_err(|e| ProviderError::Other(e.to_string()))?;

        persist_trie_update::<tables::TrieClassNodes, tables::TrieClassLeaves, _>(
            &self.0,
            &update,
            block_number,
            TrieType::Classes,
            &mut next_idx,
        )
        .map_err(|e| ProviderError::Other(e.to_string()))?;

        Ok(root_commitment)
    }

    fn trie_insert_contract_updates(
        &self,
        block_number: BlockNumber,
        state_updates: &StateUpdates,
    ) -> ProviderResult<Felt> {
        let factory = TrieDbFactory::new(self.0.clone());

        let prev_block = if block_number > 0 { block_number - 1 } else { block_number };

        let mut contract_leafs: HashMap<ContractAddress, ContractLeaf> = HashMap::new();

        // First: insert contract storage changes
        let mut storage_next_idx = next_node_index::<tables::TrieStorageNodes, _>(&self.0)
            .map_err(|e| ProviderError::Other(e.to_string()))?;

        for (address, storage_entries) in &state_updates.storage_updates {
            let mut storage_trie = factory.storages_trie(*address, prev_block);

            for (key, value) in storage_entries {
                storage_trie
                    .insert(*key, *value)
                    .map_err(|e| ProviderError::Other(e.to_string()))?;
            }

            let update = storage_trie.commit().map_err(|e| ProviderError::Other(e.to_string()))?;

            contract_leafs.entry(*address).or_default().storage_root = Some(update.root_commitment);

            persist_storage_trie_update(
                &self.0,
                &update,
                block_number,
                *address,
                &mut storage_next_idx,
            )
            .map_err(|e| ProviderError::Other(e.to_string()))?;
        }

        // Collect nonce and class hash updates
        for (address, nonce) in &state_updates.nonce_updates {
            contract_leafs.entry(*address).or_default().nonce = Some(*nonce);
        }

        for (address, class_hash) in &state_updates.deployed_contracts {
            contract_leafs.entry(*address).or_default().class_hash = Some(*class_hash);
        }

        for (address, class_hash) in &state_updates.replaced_classes {
            contract_leafs.entry(*address).or_default().class_hash = Some(*class_hash);
        }

        // Compute leaf hashes for updated contracts
        let leaf_hashes: Vec<_> = contract_leafs
            .into_iter()
            .map(|(address, mut leaf)| {
                // Get storage root if not already set from storage updates
                if leaf.storage_root.is_none() {
                    let storage_trie = factory.storages_trie(address, prev_block);
                    let root = storage_trie
                        .root_hash()
                        .map_err(|e| ProviderError::Other(e.to_string()))?;
                    leaf.storage_root = Some(root);
                }

                let state = if block_number == 0 {
                    self.latest()?
                } else {
                    self.historical((block_number - 1).into())?
                        .expect("historical state should exist")
                };

                let leaf_hash = contract_state_leaf_hash(state, &address, &leaf);

                Ok((address, leaf_hash))
            })
            .collect::<Result<Vec<_>, ProviderError>>()?;

        // Insert leaf hashes into contracts trie
        let mut contract_trie = factory.contracts_trie(prev_block);

        for (address, leaf_hash) in leaf_hashes {
            contract_trie
                .insert(address, leaf_hash)
                .map_err(|e| ProviderError::Other(e.to_string()))?;
        }

        let update = contract_trie.commit().map_err(|e| ProviderError::Other(e.to_string()))?;

        let root_commitment = update.root_commitment;

        let mut next_idx = next_node_index::<tables::TrieContractNodes, _>(&self.0)
            .map_err(|e| ProviderError::Other(e.to_string()))?;

        persist_trie_update::<tables::TrieContractNodes, tables::TrieContractLeaves, _>(
            &self.0,
            &update,
            block_number,
            TrieType::Contracts,
            &mut next_idx,
        )
        .map_err(|e| ProviderError::Other(e.to_string()))?;

        Ok(root_commitment)
    }
}

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
