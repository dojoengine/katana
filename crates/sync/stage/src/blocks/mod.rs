use futures::future::BoxFuture;
use katana_primitives::chain::ChainId;
use katana_primitives::Felt;
use katana_provider::api::block::{BlockHashProvider, BlockWriter};
use katana_provider::{DbProviderFactory, MutableProvider, ProviderError, ProviderFactory};
use rayon::prelude::*;
use tracing::{error, info_span, warn, Instrument};

use crate::{
    PruneInput, PruneOutput, PruneResult, Stage, StageExecutionInput, StageExecutionOutput,
    StageResult,
};

mod downloader;
pub mod hash;

pub use downloader::json_rpc::JsonRpcBlockDownloader;
pub use downloader::{BatchBlockDownloader, BlockData, BlockDownloader};

/// A stage for syncing blocks.
#[derive(Debug)]
pub struct Blocks<B> {
    provider: DbProviderFactory,
    downloader: B,
    chain_id: ChainId,
}

impl<B> Blocks<B> {
    /// Create a new [`Blocks`] stage.
    pub fn new(provider: DbProviderFactory, downloader: B, chain_id: ChainId) -> Self {
        Self { provider, downloader, chain_id }
    }

    /// Validates that the downloaded blocks form a valid chain.
    ///
    /// This method checks the chain invariant: block N's parent hash must be block N-1's hash.
    /// For the first block in the list (if not block 0), it fetches the parent hash from storage.
    fn validate_chain_invariant(&self, blocks: &[BlockData]) -> Result<(), Error> {
        if blocks.is_empty() {
            return Ok(());
        }

        let first_block = &blocks[0].block.block;
        let first_block_num = first_block.header.number;

        if first_block_num > 0 {
            let parent_block_num = first_block_num - 1;
            let expected_parent_hash = self
                .provider
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
}

impl<D> Stage for Blocks<D>
where
    D: BlockDownloader,
{
    fn id(&self) -> &'static str {
        "Blocks"
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

            // TODO: spawn onto a blocking thread pool
            self.validate_chain_invariant(&blocks)?;

            // Phase 1: Compute commitments and verify hashes in parallel.
            // These are CPU-bound hash computations with no inter-block dependencies.
            let mut blocks = blocks;
            blocks.par_iter_mut().for_each(|block_data| {
                let block_number = block_data.block.block.header.number;

                // Compute missing commitments for older blocks where the source
                // doesn't include them in the block header.
                hash::compute_missing_commitments(
                    &mut block_data.block.block,
                    &block_data.receipts,
                    &block_data.state_updates.state_updates,
                );

                // Verify the block hash matches what we compute locally.
                let computed_hash =
                    hash::compute_hash(&block_data.block.block.header, &self.chain_id);

                if computed_hash != block_data.block.block.hash {
                    warn!(
                        block = %block_number,
                        expected = %format!("{:#x}", block_data.block.block.hash),
                        computed = %format!("{:#x}", computed_hash),
                        "Block hash mismatch"
                    );
                }
            });

            // Phase 2: Write blocks to the database sequentially.
            let provider_mut = self.provider.provider_mut();

            for block_data in blocks {
                let BlockData { block, receipts, state_updates } = block_data;
                let block_number = block.block.header.number;

                provider_mut
                    .insert_block_with_states_and_receipts(
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

            Ok(StageExecutionOutput { last_block_processed: input.to() })
        })
    }

    // TODO: implement block pruning
    fn prune<'a>(&'a mut self, input: &'a PruneInput) -> BoxFuture<'a, PruneResult> {
        let _ = input;
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

    #[error(
        "chain invariant violation: block {block_num} parent hash {parent_hash:#x} does not match \
         previous block hash {expected_hash:#x}"
    )]
    ChainInvariantViolation { block_num: u64, parent_hash: Felt, expected_hash: Felt },

    #[error(
        "block hash mismatch: block {block_num} gateway hash {expected:#x} does not match \
         computed hash {computed:#x}"
    )]
    BlockHashMismatch { block_num: u64, expected: Felt, computed: Felt },
}
