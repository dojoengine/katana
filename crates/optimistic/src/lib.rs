use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use backon::{ExponentialBuilder, Retryable};
use katana_primitives::env::BlockEnv;
use katana_tasks::TaskSpawner;
use parking_lot::Mutex;
use starknet::core::types::{BlockId, BlockTag, MaybePendingBlockWithTxs};
use starknet::providers::{jsonrpc::HttpTransport, JsonRpcClient};
use starknet::providers::{Provider, ProviderError};

/// global block context used by the execution engine for executing proxied transactions
#[derive(Debug, Clone)]
pub struct UpstreamBlockContext {
    inner: Arc<Mutex<BlockEnv>>,
}

#[derive(Debug)]
pub struct BlockSyncer {
    task_spawner: TaskSpawner,
    client: Arc<JsonRpcClient<HttpTransport>>,
    current_block_context: UpstreamBlockContext,
}

impl BlockSyncer {
    pub async fn initialize(&self) -> Result<()> {
        let client = self.client.clone();
        let current_block_ctx = self.current_block_context.clone();

        // Fetch the latest confirmed block to get the base number.
        let latest_req =
            || async { client.get_block_with_txs(BlockId::Tag(BlockTag::Latest)).await };
        let latest_block = latest_req.retry(ExponentialBuilder::default()).await?;

        let latest_confirmed_number = match latest_block {
            MaybePendingBlockWithTxs::Block(b) => b.block_number,
            // According to Starknet spec, BlockTag::Latest should return a Block, not PendingBlock.
            // If it does return PendingBlock, it's an unexpected state from the provider.
            MaybePendingBlockWithTxs::PendingBlock(_) => {
                return Err(anyhow::anyhow!("Provider returned PendingBlock for BlockTag::Latest"));
            }
        };

        // Fetch the pending block information to get the sequencer address and other pending details.
        let pending_req =
            || async { client.get_block_with_txs(BlockId::Tag(BlockTag::Pending)).await };
        let pending_block = pending_req.retry(ExponentialBuilder::default()).await?;

        // Lock the mutex and update the block context.
        let mut block_ctx = current_block_ctx.inner.lock();

        // The context should represent the state of the *next* block or the current pending one.
        // If `Pending` returns a `PendingBlock`, its number is `latest_confirmed_number + 1`.
        // If `Pending` returns a `Block(N)`, it means block N was just confirmed.
        // The original loop logic when getting a `Block` sets the context number to `block.block_number`.
        // Let's follow that pattern here for consistency with how the loop *might* behave later.
        match pending_block {
            MaybePendingBlockWithTxs::Block(block) => {
                // If Pending returns a Block (e.g., N), context is for this confirmed block or N+1?
                // Original loop logic: block_ctx.number = block.block_number;
                // Let's follow this pattern: context number is the number of the confirmed block.
                block_ctx.number = block.block_number;
                block_ctx.sequencer_address = block.sequencer_address.into();
                block_ctx.timestamp = block.timestamp;
                // block_ctx.l1_gas_prices = block.l1_gas_prices.into();
            }
            MaybePendingBlockWithTxs::PendingBlock(pending) => {
                // If Pending returns a PendingBlock, the context is for the next block, which is latest_confirmed + 1.
                block_ctx.number = latest_confirmed_number + 1;
                block_ctx.sequencer_address = pending.sequencer_address.into();
                block_ctx.timestamp = pending.timestamp;
                // block_ctx.l1_gas_prices = pending.l1_gas_prices.into(); / Assuming BlockEnv needs gas prices
            }
        }

        Ok(())
    }

    pub fn start_background_sync(&self) -> Result<()> {
        let client = self.client.clone();
        let task_spawner = self.task_spawner.clone();
        let current_block_ctx = self.current_block_context.clone();

        task_spawner.build_task().spawn(async move {
            // This loop continues syncing the block context periodically.
            // It uses the original logic provided in the snippet.
            let req = || async { client.get_block_with_txs(BlockId::Tag(BlockTag::Pending)).await };

            loop {
                // Keep original loop structure and logic, including the `?` error handling
                // which will cause the task to exit on an unrecoverable error.
                let block = req.retry(ExponentialBuilder::default()).await?; // Exit task on error

                match block {
                    MaybePendingBlockWithTxs::Block(block) => {
                        let mut block_ctx = current_block_ctx.inner.lock();
                        block_ctx.number = block.block_number;
                        block_ctx.timestamp = block.timestamp; // Assuming BlockEnv needs timestamp
                        block_ctx.sequencer_address = block.sequencer_address.into();
                        // block_ctx.l1_gas_prices = block.l1_gas_prices.into(); // Assuming BlockEnv needs gas prices
                    }

                    MaybePendingBlockWithTxs::PendingBlock(pending) => {
                        let mut block_ctx = current_block_ctx.inner.lock();
                        // Original logic: increments from the *current* context number.
                        // This logic might be questionable depending on the exact state
                        // held by `block_ctx.number`, but is kept as per the original snippet.
                        block_ctx.number += 1;
                        block_ctx.timestamp = pending.timestamp; // Assuming BlockEnv needs timestamp
                        block_ctx.sequencer_address = pending.sequencer_address.into();
                        // block_ctx.l1_gas_prices = pending.l1_gas_prices.into(); // Assuming BlockEnv needs gas prices
                    }
                }

                tokio::time::sleep(Duration::from_secs(2)).await;
            }

            // Keep original unreachable code structure and return type.
            #[allow(unreachable_code)]
            Result::<(), ProviderError>::Ok(())
        });

        Ok(())
    }
}
