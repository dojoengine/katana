use std::collections::BTreeMap;

use katana_db::abstraction::Database;
use katana_primitives::block::BlockNumber;
use katana_primitives::class::{ClassHash, CompiledClassHash};
use katana_primitives::state::StateUpdates;
use katana_primitives::Felt;
use katana_provider_api::trie::TrieWriter;

use super::ForkedProvider;
use crate::ProviderResult;

impl<Db: Database> TrieWriter for ForkedProvider<Db> {
    fn trie_insert_contract_updates(
        &self,
        block_number: BlockNumber,
        state_updates: &StateUpdates,
    ) -> ProviderResult<Felt> {
        let _ = block_number;
        let _ = state_updates;
        Ok(Felt::ZERO)
    }

    fn trie_insert_declared_classes(
        &self,
        block_number: BlockNumber,
        updates: &BTreeMap<ClassHash, CompiledClassHash>,
    ) -> ProviderResult<Felt> {
        let _ = block_number;
        let _ = updates;
        Ok(Felt::ZERO)
    }
}
