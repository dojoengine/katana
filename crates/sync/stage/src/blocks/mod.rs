use anyhow::Result;
use futures::future::BoxFuture;
use katana_primitives::chain::ChainId;
use katana_primitives::Felt;
use katana_provider::api::block::BlockHashProvider;
use katana_provider::{DbProviderFactory, MutableProvider, ProviderError, ProviderFactory};
use katana_tasks::TaskSpawner;
use rayon::prelude::*;
use tracing::{error, info_span, Instrument};

use crate::{
    PruneInput, PruneOutput, PruneResult, Stage, StageExecutionInput, StageExecutionOutput,
    StageResult,
};

mod downloader;
pub mod hash;

pub use downloader::json_rpc::JsonRpcBlockDownloader;
pub use downloader::{BatchBlockDownloader, BlockData, BlockDownloader};

pub const BLOCKS_STAGE_ID: &str = "Blocks";

/// A stage for syncing blocks.
#[derive(Debug)]
pub struct Blocks<B> {
    provider: DbProviderFactory,
    downloader: B,
    chain_id: ChainId,
    task_spawner: TaskSpawner,
}

impl<B> Blocks<B> {
    /// Create a new [`Blocks`] stage.
    pub fn new(
        provider: DbProviderFactory,
        downloader: B,
        chain_id: ChainId,
        task_spawner: TaskSpawner,
    ) -> Self {
        Self { provider, downloader, chain_id, task_spawner }
    }
}

/// Validates that the downloaded blocks form a valid chain.
///
/// Checks the chain invariant: block N's parent hash must be block N-1's hash.
/// For the first block in the list (if not block 0), it fetches the parent hash from storage.
fn validate_chain_invariant(
    provider: &DbProviderFactory,
    blocks: &[BlockData],
) -> Result<(), Error> {
    if blocks.is_empty() {
        return Ok(());
    }

    let first_block = &blocks[0].block.block;
    let first_block_num = first_block.header.number;

    if first_block_num > 0 {
        let parent_block_num = first_block_num - 1;
        let expected_parent_hash = provider
            .provider()
            .block_hash_by_num(parent_block_num)?
            .ok_or(ProviderError::MissingBlockHash(parent_block_num))?;

        if first_block.header.parent_hash != expected_parent_hash {
            return Err(Error::ChainInvariantViolation {
                block_num: first_block_num,
                parent_hash: first_block.header.parent_hash,
                expected_hash: expected_parent_hash,
            });
        }
    }

    for window in blocks.windows(2) {
        let prev_block = &window[0].block.block;
        let curr_block = &window[1].block.block;

        let prev_hash = prev_block.hash;
        let curr_block_num = curr_block.header.number;

        if curr_block.header.parent_hash != prev_hash {
            return Err(Error::ChainInvariantViolation {
                block_num: curr_block_num,
                parent_hash: curr_block.header.parent_hash,
                expected_hash: prev_hash,
            });
        }
    }

    Ok(())
}

impl<D> Stage for Blocks<D>
where
    D: BlockDownloader,
{
    fn id(&self) -> &'static str {
        BLOCKS_STAGE_ID
    }

    fn execute<'a>(&'a mut self, input: &'a StageExecutionInput) -> BoxFuture<'a, StageResult> {
        Box::pin(async move {
            let blocks = self
                .downloader
                .download_blocks(input.from(), input.to())
                .instrument(info_span!(target: "stage", "blocks.download", from = %input.from(), to = %input.to()))
                .await
                .map_err(|e| Error::Download(Box::new(e)))?;

            let span = info_span!(target: "stage", "blocks.insert", from = %input.from(), to = %input.to());
            let _enter = span.enter();

            // Validate chain invariant and compute commitments/hashes in parallel on the CPU pool.
            let chain_id = self.chain_id;
            let provider = self.provider.clone();
            let mut blocks = self
                .task_spawner
                .cpu_bound()
                .spawn(move || {
                    validate_chain_invariant(&provider, &blocks)?;

                    let mut blocks = blocks;
                    blocks.par_iter_mut().try_for_each(|block_data| {
                        let block_hash = block_data.block.block.hash;
                        let block_num = block_data.block.block.header.number;

                        let verified = hash::patch_and_verify_block_hash(
                            &mut block_data.block.block,
                            &block_data.receipts,
                            &block_data.state_updates.state_updates,
                            &chain_id,
                        );

                        if verified {
                            Ok(())
                        } else {
                            Err(Error::BlockVerificationFailed {
                                block_num,
                                expected_block_hash: block_hash,
                            })
                        }
                    })?;

                    Result::<_, Error>::Ok(blocks)
                })
                .await
                .map_err(Error::TaskJoinError)??;

            // Write blocks to the database sequentially.
            let provider = self.provider.clone();
            self.task_spawner
                .spawn_blocking(move || {
                    let provider_mut = provider.provider_mut();

                    for block_data in blocks.drain(..) {
                        let BlockData { block, receipts, state_updates } = block_data;
                        let block_number = block.block.header.number;

                        provider_mut
                            .insert_block_data(
                                block,
                                state_updates,
                                receipts,
                                Vec::new(),
                            )
                            .inspect_err(
                                |e| error!(error = %e, block = %block_number, "Error storing block."),
                            )?;
                    }

                    provider_mut.commit()?;
                    Result::<(), Error>::Ok(())
                })
                .await
                .map_err(Error::TaskJoinError)??;

            Ok(StageExecutionOutput { last_block_processed: input.to() })
        })
    }

    fn prune<'a>(&'a mut self, _input: &'a PruneInput) -> BoxFuture<'a, PruneResult> {
        Box::pin(async move { Ok(PruneOutput::default()) })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Error returned by the block downloader.
    #[error(transparent)]
    Download(Box<dyn std::error::Error + Send + Sync>),

    #[error(transparent)]
    Provider(#[from] ProviderError),

    #[error(transparent)]
    Database(#[from] katana_db::error::DatabaseError),

    #[error(
        "chain invariant violation: block {block_num} parent hash {parent_hash:#x} does not match \
         previous block hash {expected_hash:#x}"
    )]
    ChainInvariantViolation { block_num: u64, parent_hash: Felt, expected_hash: Felt },

    #[error("block hash verification failed: block {block_num} hash {expected_block_hash:#x}")]
    BlockVerificationFailed { block_num: u64, expected_block_hash: Felt },

    #[error("task join error: {0}")]
    TaskJoinError(katana_tasks::JoinError),
}
