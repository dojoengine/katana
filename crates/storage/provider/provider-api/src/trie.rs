use katana_primitives::block::BlockNumber;
use katana_primitives::class::{ClassHash, CompiledClassHash};
use katana_primitives::hash::StarkHash;
use katana_primitives::state::StateUpdates;
use katana_primitives::Felt;
use starknet::macros::short_string;

use crate::ProviderResult;

#[auto_impl::auto_impl(&, Box, Arc)]
pub trait TrieWriter: Send + Sync {
    fn trie_insert_declared_classes(
        &self,
        block_number: BlockNumber,
        classes: Vec<(ClassHash, CompiledClassHash)>,
    ) -> ProviderResult<Felt>;

    fn trie_insert_contract_updates(
        &self,
        block_number: BlockNumber,
        state_updates: &StateUpdates,
    ) -> ProviderResult<Felt>;

    /// Compute state root for a block with given state updates.
    /// Can be overridden by providers that need special logic (e.g., ForkedProvider with partial
    /// tries).
    fn compute_state_root(
        &self,
        block_number: BlockNumber,
        state_updates: &StateUpdates,
    ) -> ProviderResult<Felt> {
        // Default implementation for regular providers
        let class_trie_root = self.trie_insert_declared_classes(
            block_number,
            state_updates.declared_classes.clone().into_iter().collect(),
        )?;

        let contract_trie_root = self.trie_insert_contract_updates(block_number, state_updates)?;

        Ok(starknet_types_core::hash::Poseidon::hash_array(&[
            short_string!("STARKNET_STATE_V0"),
            contract_trie_root,
            class_trie_root,
        ]))
    }
}
