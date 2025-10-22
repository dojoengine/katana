use futures::future::BoxFuture;
use katana_primitives::block::BlockNumber;
use katana_primitives::Felt;
use katana_provider::api::block::HeaderProvider;
use katana_provider::api::state_update::StateUpdateProvider;
use katana_provider::api::trie::TrieWriter;
use katana_rpc_types::class;
use katana_trie::CommitId;
use starknet::macros::short_string;
use starknet_types_core::hash::{Poseidon, StarkHash};
use tracing::{debug, debug_span, error};

use crate::{Stage, StageExecutionInput, StageExecutionOutput, StageResult};

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
            for block_number in input.from()..=input.to() {
                let span = debug_span!("state_trie.compute_state_root", %block_number);
                let _enter = span.enter();

                let header = self
                    .provider
                    .header(block_number.into())?
                    .ok_or(Error::MissingBlockHeader(block_number))?;

                let expected_state_root = header.state_root;

                let state_update = self
                    .provider
                    .state_update(block_number.into())?
                    .ok_or(Error::MissingStateUpdate(block_number))?;

                let computed_contract_trie_root =
                    self.provider.trie_insert_contract_updates(block_number, &state_update)?;

                debug!(
                    contract_trie_root = format!("{computed_contract_trie_root:#x}"),
                    "Computed contract trie root."
                );

                let computed_class_trie_root = self
                    .provider
                    .trie_insert_declared_classes(block_number, &state_update.declared_classes)?;

                debug!(
                    classes_tri_root = format!("{computed_class_trie_root:#x}"),
                    "Computed classes trie root."
                );

                let computed_state_root = if dbg!(computed_class_trie_root == Felt::ZERO) {
                    computed_contract_trie_root
                } else {
                    Poseidon::hash_array(&[
                        short_string!("STARKNET_STATE_V0"),
                        computed_contract_trie_root,
                        computed_class_trie_root,
                    ])
                };

                // Verify that the computed state root matches the expected state root from the
                // block header
                if computed_state_root != expected_state_root {
                    error!(
                        target: "stage",
                        state_root = %format!("{computed_state_root:#x}"),
                        expected_state_root = %format!("{expected_state_root:#x}"),
                        "Bad state trie root for block - computed state root does not match expected state root (from header)",
                    );

                    return Err(Error::StateRootMismatch {
                        block_number,
                        expected: expected_state_root,
                        computed: computed_state_root,
                    }
                    .into());
                }

                debug!("State root verified successfully.");
            }

            Ok(StageExecutionOutput { last_block_processed: input.to() })
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
        "State root mismatch at block {block_number}: expected (from header) {expected:#x}, \
         computed {computed:#x}"
    )]
    StateRootMismatch { block_number: BlockNumber, expected: Felt, computed: Felt },
}
