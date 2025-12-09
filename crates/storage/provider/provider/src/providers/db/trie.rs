use std::collections::{BTreeMap, HashMap};

use katana_db::abstraction::DbTxMut;
use katana_db::tables;
use katana_db::trie::TrieDbMut;
use katana_primitives::block::BlockNumber;
use katana_primitives::class::{ClassHash, CompiledClassHash};
use katana_primitives::state::StateUpdates;
use katana_primitives::{ContractAddress, Felt};
use katana_provider_api::state::{StateFactoryProvider, StateProvider};
use katana_provider_api::trie::TrieWriter;
use katana_provider_api::ProviderError;
use katana_trie::{
    compute_contract_state_hash, ClassesTrie, ContractLeaf, ContractsTrie, StoragesTrie,
};

use crate::providers::db::DbProvider;
use crate::ProviderResult;

impl<Tx: DbTxMut> TrieWriter for DbProvider<Tx> {
    fn trie_insert_declared_classes(
        &self,
        block_number: BlockNumber,
        updates: &BTreeMap<ClassHash, CompiledClassHash>,
    ) -> ProviderResult<Felt> {
        let mut trie = ClassesTrie::new(TrieDbMut::<tables::ClassesTrie, _>::new(self.0.clone()));

        for (class_hash, compiled_hash) in updates {
            trie.insert(*class_hash, *compiled_hash);
        }

        trie.commit(block_number);
        Ok(trie.root())
    }

    fn trie_insert_contract_updates(
        &self,
        block_number: BlockNumber,
        state_updates: &StateUpdates,
    ) -> ProviderResult<Felt> {
        let mut contract_trie_db =
            ContractsTrie::new(TrieDbMut::<tables::ContractsTrie, _>::new(self.0.clone()));

        let mut contract_leafs: HashMap<ContractAddress, ContractLeaf> = HashMap::new();

        let leaf_hashes: Vec<_> = {
            // First we insert the contract storage changes
            for (address, storage_entries) in &state_updates.storage_updates {
                let mut storage_trie_db = StoragesTrie::new(
                    TrieDbMut::<tables::StoragesTrie, _>::new(self.0.clone()),
                    *address,
                );

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
                        TrieDbMut::<tables::StoragesTrie, _>::new(self.0.clone()),
                        address,
                    );
                    let storage_root = storage_trie.root();
                    leaf.storage_root = Some(storage_root);

                    let state = if block_number == 0 {
                        self.latest()? // this will just default to an empty state
                    } else {
                        self.historical((block_number - 1).into())?
                            .expect("historical state should exist")
                    };

                    let leaf_hash = contract_state_leaf_hash(state, &address, &leaf);

                    Ok((address, leaf_hash))
                })
                .collect::<Result<Vec<_>, ProviderError>>()?
        };

        for (k, v) in leaf_hashes {
            contract_trie_db.insert(k, v);
        }

        contract_trie_db.commit(block_number);
        Ok(contract_trie_db.root())
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
