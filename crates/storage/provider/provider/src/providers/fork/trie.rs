use std::collections::BTreeMap;

use katana_db::abstraction::{Database, DbTxMut};
use katana_primitives::block::BlockNumber;
use katana_primitives::class::{ClassHash, CompiledClassHash};
use katana_primitives::state::StateUpdates;
use katana_primitives::Felt;
use katana_provider_api::trie::TrieWriter;

use super::ForkedProvider;
use crate::ProviderResult;

impl<Tx> TrieWriter for ForkedProvider<Tx>
where
    Tx: DbTxMut + Send + Sync,
{
    fn trie_insert_contract_updates(
        &self,
        block_number: BlockNumber,
        state_updates: &StateUpdates,
    ) -> ProviderResult<Felt> {
        self.provider.trie_insert_contract_updates(block_number, state_updates)
    }

    fn trie_insert_declared_classes(
        &self,
        block_number: BlockNumber,
        updates: &BTreeMap<ClassHash, CompiledClassHash>,
    ) -> ProviderResult<Felt> {
        self.provider.trie_insert_declared_classes(block_number, updates)
    }
}
