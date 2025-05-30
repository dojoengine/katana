use std::collections::BTreeMap;

use katana_primitives::block::BlockNumber;
use katana_primitives::class::{ClassHash, CompiledClassHash};
use katana_primitives::hash::StarkHash;
use katana_primitives::state::StateUpdates;
use katana_primitives::Felt;
use katana_trie::MultiProof;

use crate::ProviderResult;

#[auto_impl::auto_impl(&, Box, Arc)]
pub trait TrieWriter: Send + Sync {
    fn trie_insert_declared_classes(
        &self,
        block_number: BlockNumber,
        updates: &BTreeMap<ClassHash, CompiledClassHash>,
    ) -> ProviderResult<Felt>;

    fn trie_insert_contract_updates(
        &self,
        block_number: BlockNumber,
        state_updates: &StateUpdates,
    ) -> ProviderResult<Felt>;

    // New methods for partial tries - with default implementations
    fn trie_insert_declared_classes_with_proof(
        &self,
        block_number: BlockNumber,
        updates: &BTreeMap<ClassHash, CompiledClassHash>,
        _proof: MultiProof,
        _original_root: Felt,
    ) -> ProviderResult<Felt> {
        // Default implementation falls back to regular method (ignoring proof)
        self.trie_insert_declared_classes(block_number, updates)
    }

    fn trie_insert_contract_updates_with_proof(
        &self,
        block_number: BlockNumber,
        state_updates: &StateUpdates,
        _proof: MultiProof,
        _original_root: Felt,
    ) -> ProviderResult<Felt> {
        // Default implementation falls back to regular method (ignoring proof)
        self.trie_insert_contract_updates(block_number, state_updates)
    }

    /// Compute state root - can be overridden by providers that need special logic
    fn compute_state_root(
        &self,
        block_number: BlockNumber,
        state_updates: &StateUpdates,
    ) -> ProviderResult<Felt> {
        // Default implementation for regular providers
        let class_trie_root =
            self.trie_insert_declared_classes(block_number, &state_updates.declared_classes)?;
        let contract_trie_root = self.trie_insert_contract_updates(block_number, state_updates)?;

        Ok(starknet_types_core::hash::Poseidon::hash_array(&[
            starknet::macros::short_string!("STARKNET_STATE_V0"),
            contract_trie_root,
            class_trie_root,
        ]))
    }
}
