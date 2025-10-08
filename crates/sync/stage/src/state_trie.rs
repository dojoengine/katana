use futures::future::BoxFuture;
use katana_primitives::block::{BlockHashOrNumber, BlockNumber};
use katana_primitives::Felt;
use katana_provider::api::block::HeaderProvider;
use katana_provider::api::state_update::StateUpdateProvider;
use katana_provider::api::trie::TrieWriter;
use starknet::macros::short_string;
use starknet_types_core::hash::{Poseidon, StarkHash};
use tracing::debug;

use crate::{Stage, StageExecutionInput, StageResult};

/// A stage for computing and validating state tries.
///
/// This stage processes blocks that have been stored by the [`Blocks`](crate::blocks::Blocks)
/// stage and computes the state root for each block by applying the state updates to the trie.
///
/// The stage fetches the state update for each block in the input range and inserts the updates
/// into the contract and class tries via the [`TrieWriter`] trait, which computes the new state
/// root.
#[derive(Debug)]
pub struct StateTrie<P> {
    provider: P,
}

impl<P> StateTrie<P> {
    /// Create a new [`StateTrie`] stage.
    pub fn new(provider: P) -> Self {
        Self { provider }
    }
}

impl<P> Stage for StateTrie<P>
where
    P: StateUpdateProvider + TrieWriter + HeaderProvider,
{
    fn id(&self) -> &'static str {
        "StateTrie"
    }

    fn execute<'a>(&'a mut self, input: &'a StageExecutionInput) -> BoxFuture<'a, StageResult> {
        Box::pin(async move {
            debug!(
                target: "stage",
                id = %self.id(),
                from = %input.from(),
                to = %input.to(),
                "Computing state roots for blocks."
            );

            for block_number in input.from()..=input.to() {
                // Fetch the block header to get the expected state root
                let header = self
                    .provider
                    .header(BlockHashOrNumber::Num(block_number))?
                    .ok_or_else(|| Error::MissingBlockHeader(block_number))?;

                // Fetch the state update for this block
                let state_update = self
                    .provider
                    .state_update(BlockHashOrNumber::Num(block_number))?
                    .ok_or_else(|| Error::MissingStateUpdate(block_number))?;

                // Compute the state root by inserting the state updates into the tries
                // This mirrors what `compute_new_state_root` does in the backend

                // Insert declared classes into the class trie
                let class_trie_root = self
                    .provider
                    .trie_insert_declared_classes(block_number, &state_update.declared_classes)?;

                // Insert contract updates into the contract trie
                let contract_trie_root =
                    self.provider.trie_insert_contract_updates(block_number, &state_update)?;

                // Compute the state root: hash("STARKNET_STATE_V0", contract_trie_root,
                // class_trie_root)
                let computed_state_root = Poseidon::hash_array(&[
                    short_string!("STARKNET_STATE_V0"),
                    contract_trie_root,
                    class_trie_root,
                ]);

                // Verify that the computed state root matches the expected state root from the
                // block header
                if computed_state_root != header.state_root {
                    return Err(Error::StateRootMismatch {
                        block_number,
                        expected: header.state_root,
                        computed: computed_state_root,
                    }
                    .into());
                }

                debug!(
                    target: "stage",
                    id = %self.id(),
                    block_number = %block_number,
                    state_root = %computed_state_root,
                    "State root verified successfully."
                );
            }

            debug!(
                target: "stage",
                id = %self.id(),
                from = %input.from(),
                to = %input.to(),
                "Finished computing state roots."
            );

            Ok(())
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Missing block header for block {0}")]
    MissingBlockHeader(BlockNumber),

    #[error("Missing state update for block {0}")]
    MissingStateUpdate(BlockNumber),

    #[error(
        "State root mismatch at block {block_number}: expected {expected:#x}, computed \
         {computed:#x}"
    )]
    StateRootMismatch { block_number: BlockNumber, expected: Felt, computed: Felt },
}
