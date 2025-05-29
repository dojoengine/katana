use super::ForkedProvider;
use crate::error::ProviderError;
use crate::providers::db::DbProvider;
use crate::traits::trie::TrieWriter;
use crate::ProviderResult;
use katana_db::abstraction::Database;
use katana_db::tables;
use katana_db::trie::TrieDbMut;
use katana_primitives::block::BlockNumber;
use katana_primitives::class::{ClassHash, CompiledClassHash};
use katana_primitives::state::StateUpdates;
use katana_primitives::{ContractAddress, Felt};
use katana_trie::{
    compute_contract_state_hash, ClassesTrie, ContractLeaf, ContractsTrie, StoragesTrie,
};
use std::collections::{BTreeMap, HashMap};

impl<Db: Database> TrieWriter for ForkedProvider<Db> {
    fn trie_insert_contract_updates(
        &self,
        block_number: BlockNumber,
        state_updates: &StateUpdates,
    ) -> ProviderResult<Felt> {
        self.0.update(|tx| {
            let mut trie = ClassesTrie::new_partial(TrieDbMut::<tables::ClassesTrie, _>::new(tx));

            for (class_hash, compiled_hash) in updates {
                trie.insert(*class_hash, *compiled_hash, proof, original_root);
            }

            trie.commit(block_number);
            Ok(trie.root())
        })?
    }

    fn trie_insert_declared_classes(
        &self,
        block_number: BlockNumber,
        updates: &BTreeMap<ClassHash, CompiledClassHash>,
    ) -> ProviderResult<Felt> {
        self.0.update(|tx| {
            let mut contract_trie_db =
                ContractsTrie::new_partial(TrieDbMut::<tables::ContractsTrie, _>::new(tx));

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

                        let latest_state = self.latest()?;
                        let leaf_hash = contract_state_leaf_hash(latest_state, &address, &leaf);

                        Ok((address, leaf_hash))
                    })
                    .collect::<Result<Vec<_>, ProviderError>>()?
            };

            for (k, v) in leaf_hashes {
                contract_trie_db.insert(k, v, proof, original_root);
            }

            contract_trie_db.commit(block_number);
            Ok(contract_trie_db.root())
        })?
    }
}
