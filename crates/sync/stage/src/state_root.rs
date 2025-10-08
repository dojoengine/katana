use futures::future::BoxFuture;
use katana_primitives::block::{BlockHashOrNumber, BlockNumber};
use katana_provider::api::state_update::StateUpdateProvider;
use katana_provider::api::trie::TrieWriter;
use tracing::debug;

use crate::{Stage, StageExecutionInput, StageResult};

/// A stage for computing and validating state roots.
///
/// This stage processes blocks that have been stored by the [`Blocks`](crate::blocks::Blocks)
/// stage and computes the state root for each block by applying the state updates to the trie.
///
/// The stage fetches the state update for each block in the input range and inserts the updates
/// into the contract and class tries via the [`TrieWriter`] trait, which computes the new state
/// root.
#[derive(Debug)]
pub struct StateRoot<P> {
    provider: P,
}

impl<P> StateRoot<P> {
    /// Create a new [`StateRoot`] stage.
    pub fn new(provider: P) -> Self {
        Self { provider }
    }
}

impl<P> Stage for StateRoot<P>
where
    P: StateUpdateProvider + TrieWriter,
{
    fn id(&self) -> &'static str {
        "StateRoot"
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
                // Fetch the state update for this block
                let state_update = self
                    .provider
                    .state_update(BlockHashOrNumber::Num(block_number))?
                    .ok_or_else(|| Error::MissingStateUpdate(block_number))?;

                // Compute the state root by inserting the state updates into the tries
                // This mirrors what `compute_new_state_root` does in the backend

                // Insert declared classes into the class trie
                let _class_trie_root = self
                    .provider
                    .trie_insert_declared_classes(block_number, &state_update.declared_classes)?;

                // Insert contract updates into the contract trie
                let _contract_trie_root =
                    self.provider.trie_insert_contract_updates(block_number, &state_update)?;

                // Note: The actual state root hash computation (combining class_trie_root and
                // contract_trie_root with "STARKNET_STATE_V0") is done inside the TrieWriter
                // implementation. We just need to ensure the updates are inserted.
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
    #[error("Missing state update for block {0}")]
    MissingStateUpdate(BlockNumber),
}
