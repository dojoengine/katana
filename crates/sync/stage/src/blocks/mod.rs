use futures::future::BoxFuture;
use katana_primitives::block::SealedBlock;
use katana_primitives::chain::ChainId;
use katana_primitives::receipt::{Receipt, ReceiptWithTxHash};
use katana_primitives::version::StarknetVersion;
use katana_primitives::Felt;
use katana_provider::api::block::{BlockHashProvider, BlockWriter};
use katana_provider::{DbProviderFactory, MutableProvider, ProviderError, ProviderFactory};
use katana_trie::compute_merkle_root;
use starknet_types_core::hash::{Pedersen, Poseidon, StarkHash};
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

            let provider_mut = self.provider.provider_mut();

            for block_data in blocks {
                let BlockData { mut block, receipts, state_updates } = block_data;
                let block_number = block.block.header.number;

                // Compute missing commitments for older blocks where the source
                // doesn't include them in the block header.
                compute_missing_commitments(&mut block.block, &receipts);

                // Verify the block hash matches what we compute locally.
                let computed_hash = hash::compute_hash(&block.block.header, &self.chain_id);
                if computed_hash != block.block.hash {
                    warn!(
                        block = %block_number,
                        expected = %block.block.hash,
                        computed = %computed_hash,
                        "Block hash mismatch"
                    );
                }

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

/// Computes missing block commitments that older blocks don't include.
///
/// For blocks before 0.13.2, the source may return zero for transaction and event
/// commitments - which is the case for the Starknet feeder gateway. For receipt commitment, the
/// source may not include it at all for older blocks. This function computes them locally so that
/// block hash verification can succeed.
fn compute_missing_commitments(block: &mut SealedBlock, receipts: &[Receipt]) {
    let version = block.header.starknet_version;

    // Transaction commitment: older blocks (pre-0.7 and 0.7.0) return zero from the gateway.
    // Pre-0.13.2 uses Pedersen, post-0.13.2 uses Poseidon.
    if block.header.transactions_commitment == Felt::ZERO {
        let tx_hashes: Vec<Felt> = block.body.iter().map(|t| t.hash).collect();
        block.header.transactions_commitment = if version < StarknetVersion::V0_13_2 {
            compute_merkle_root::<Pedersen>(&tx_hashes).unwrap()
        } else {
            compute_merkle_root::<Poseidon>(&tx_hashes).unwrap()
        };
    }

    // Event commitment: older blocks return zero from the gateway.
    // Pre-0.13.2 uses Pedersen, post-0.13.2 uses Poseidon (and includes tx_hash in leaf).
    if block.header.events_commitment == Felt::ZERO {
        let event_hashes: Vec<Felt> = if version < StarknetVersion::V0_13_2 {
            receipts
                .iter()
                .flat_map(|r| {
                    r.events().iter().map(|event| {
                        let keys_hash = Pedersen::hash_array(&event.keys);
                        let data_hash = Pedersen::hash_array(&event.data);
                        Pedersen::hash_array(&[event.from_address.into(), keys_hash, data_hash])
                    })
                })
                .collect()
        } else {
            receipts
                .iter()
                .zip(block.body.iter())
                .flat_map(|(receipt, tx)| {
                    receipt.events().iter().map(move |event| {
                        let keys_hash = Poseidon::hash_array(&event.keys);
                        let data_hash = Poseidon::hash_array(&event.data);
                        Poseidon::hash_array(&[
                            tx.hash,
                            event.from_address.into(),
                            keys_hash,
                            data_hash,
                        ])
                    })
                })
                .collect()
        };

        block.header.events_commitment = if version < StarknetVersion::V0_13_2 {
            compute_merkle_root::<Pedersen>(&event_hashes).unwrap()
        } else {
            compute_merkle_root::<Poseidon>(&event_hashes).unwrap()
        };
    }

    // Receipt commitment: only used in post-0.13.2 block hashes. The source may not
    // include it, so we compute it when missing.
    if block.header.receipts_commitment == Felt::ZERO && version >= StarknetVersion::V0_13_2 {
        let receipt_hashes: Vec<Felt> = receipts
            .iter()
            .zip(block.body.iter())
            .map(|(receipt, tx)| ReceiptWithTxHash::new(tx.hash, receipt.clone()).compute_hash())
            .collect();
        block.header.receipts_commitment =
            compute_merkle_root::<Poseidon>(&receipt_hashes).unwrap();
    }
}
