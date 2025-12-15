use futures::future::BoxFuture;
use katana_db::abstraction::{Database, DbTx};
use katana_db::tables;
use katana_db::trie::TrieDbMut;
use katana_primitives::block::BlockNumber;
use katana_primitives::Felt;
use katana_provider::api::block::HeaderProvider;
use katana_provider::api::state_update::StateUpdateProvider;
use katana_provider::api::trie::TrieWriter;
use katana_provider::{DbProviderFactory, MutableProvider, ProviderFactory};
use katana_tasks::TaskSpawner;
use starknet::macros::short_string;
use starknet_types_core::hash::{Poseidon, StarkHash};
use tracing::{debug, debug_span, error};

use crate::{
    PruneInput, PruneOutput, PruneResult, Stage, StageExecutionInput, StageExecutionOutput,
    StageResult,
};

/// A stage for computing and validating state tries.
///
/// This stage processes blocks that have been stored by the [`Blocks`](crate::blocks::Blocks)
/// stage and computes the state root for each block by applying the state updates to the trie.
///
/// The stage fetches the state update for each block in the input range and inserts the updates
/// into the contract and class tries via the [`TrieWriter`] trait, which computes the new state
/// root.
#[derive(Debug)]
pub struct StateTrie {
    storage_provider: DbProviderFactory,
    task_spawner: TaskSpawner,
}

impl StateTrie {
    /// Create a new [`StateTrie`] stage.
    pub fn new(storage_provider: DbProviderFactory, task_spawner: TaskSpawner) -> Self {
        Self { storage_provider, task_spawner }
    }
}

impl Stage for StateTrie {
    fn id(&self) -> &'static str {
        "StateTrie"
    }

    fn execute<'a>(&'a mut self, input: &'a StageExecutionInput) -> BoxFuture<'a, StageResult> {
        Box::pin(async move {
            let provider_mut = self.storage_provider.provider_mut();

            for block_number in input.from()..=input.to() {
                let span = debug_span!("state_trie.compute_state_root", %block_number);
                let _enter = span.enter();

                let header = provider_mut
                    .header(block_number.into())?
                    .ok_or(Error::MissingBlockHeader(block_number))?;

                let expected_state_root = header.state_root;

                let state_update = provider_mut
                    .state_update(block_number.into())?
                    .ok_or(Error::MissingStateUpdate(block_number))?;

                let provider_mut_clone = provider_mut.clone();
                let (computed_contract_trie_root, computed_class_trie_root) =
                    self.task_spawner
                        .cpu_bound()
                        .spawn(move || {
                            let computed_contract_trie_root = provider_mut_clone
                                .trie_insert_contract_updates(block_number, &state_update)?;

                            debug!(
                                contract_trie_root = format!("{computed_contract_trie_root:#x}"),
                                "Computed contract trie root."
                            );

                            let class_updates =
                                state_update.declared_classes.clone().into_iter().chain(
                                    state_update.migrated_compiled_classes.clone().into_iter(),
                                );

                            let computed_class_trie_root = provider_mut_clone
                                .trie_insert_declared_classes(block_number, class_updates)?;

                            debug!(
                                classes_tri_root = format!("{computed_class_trie_root:#x}"),
                                "Computed classes trie root."
                            );

                            Result::<(Felt, Felt), crate::Error>::Ok((
                                computed_contract_trie_root,
                                computed_class_trie_root,
                            ))
                        })
                        .await
                        .map_err(Error::StateComputationTaskJoinError)??;

                let computed_state_root = if computed_class_trie_root == Felt::ZERO {
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
                        block = %block_number,
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

            provider_mut.commit()?;

            Ok(StageExecutionOutput { last_block_processed: input.to() })
        })
    }

    fn prune<'a>(&'a mut self, input: &'a PruneInput) -> BoxFuture<'a, PruneResult> {
        Box::pin(async move {
            let Some(range) = input.prune_range() else {
                // Archive mode, no pruning needed, or already caught up
                return Ok(PruneOutput::default());
            };

            let tx = self.storage_provider.db().tx_mut().map_err(Error::Database)?;

            let pruned_count = self
                .task_spawner
                .spawn_blocking(move || {
                    let mut pruned_count = 0u64;

                    // Remove trie snapshots for blocks in the prune range
                    for block_number in range {
                        let span = debug_span!("state_trie.prune", %block_number);
                        let _enter = span.enter();

                        // Remove snapshot from classes trie
                        let mut classes_trie_db =
                            TrieDbMut::<tables::ClassesTrie, _>::new(tx.clone());
                        classes_trie_db
                            .remove_snapshot(block_number)
                            .map_err(|e| Error::Database(e.into_inner()))?;

                        debug!(target: "stage", "Classes trie snapshot removed.");

                        // Remove snapshot from contracts trie
                        let mut contracts_trie_db =
                            TrieDbMut::<tables::ContractsTrie, _>::new(tx.clone());
                        contracts_trie_db
                            .remove_snapshot(block_number)
                            .map_err(|e| Error::Database(e.into_inner()))?;

                        debug!(target: "stage", "Contracts trie snapshot removed.");

                        // Remove snapshot from storages trie
                        let mut storages_trie_db =
                            TrieDbMut::<tables::StoragesTrie, _>::new(tx.clone());
                        storages_trie_db
                            .remove_snapshot(block_number)
                            .map_err(|e| Error::Database(e.into_inner()))?;

                        debug!(target: "stage", "Storage trie snapshot removed.");

                        pruned_count += 1;
                    }

                    tx.commit().map_err(Error::Database)?;

                    Result::<u64, Error>::Ok(pruned_count)
                })
                .await
                .map_err(Error::StateComputationTaskJoinError)??;

            debug!(target: "stage", %pruned_count, "Pruned trie snapshots");

            Ok(PruneOutput { pruned_count })
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Missing block header for block {0}")]
    MissingBlockHeader(BlockNumber),

    #[error("Missing state update for block {0}")]
    MissingStateUpdate(BlockNumber),

    #[error("State computation task join error: {0}")]
    StateComputationTaskJoinError(katana_tasks::JoinError),

    #[error(
        "State root mismatch at block {block_number}: expected (from header) {expected:#x}, \
         computed {computed:#x}"
    )]
    StateRootMismatch { block_number: BlockNumber, expected: Felt, computed: Felt },

    #[error(transparent)]
    Database(#[from] katana_db::error::DatabaseError),
}
