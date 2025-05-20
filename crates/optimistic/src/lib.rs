pub mod execution;
pub mod pool;
pub mod storage;

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use backon::{ExponentialBuilder, Retryable};
use katana_primitives::env::BlockEnv;
use katana_tasks::TaskSpawner;
use parking_lot::Mutex;
use starknet::core::types::{BlockId, BlockTag, MaybePendingBlockWithTxs};
use starknet::providers::{jsonrpc::HttpTransport, JsonRpcClient};
use starknet::providers::{Provider, ProviderError};
use tokio::sync::watch::{self, Receiver, Sender};

#[derive(Debug)]
pub struct BlockListener {
    // (prev, new)
    sender: Sender<Option<BlockEnv>>,
    task_spawner: TaskSpawner,
    client: Arc<JsonRpcClient<HttpTransport>>,
}

impl BlockListener {
    pub fn new(
        rpc_client: JsonRpcClient<HttpTransport>,
        task_spawner: TaskSpawner,
    ) -> (Self, Receiver<Option<BlockEnv>>) {
        let client = Arc::new(rpc_client);
        let (tx, rx) = watch::channel(None);
        (Self { client, task_spawner, sender: tx }, rx)
    }

    pub async fn initial_block_env(&self) -> Result<BlockEnv> {
        let latest_block = self.client.block_hash_and_number().await?;
        let current_block = self.client.get_block_with_txs(BlockId::Tag(BlockTag::Pending)).await?;

        let MaybePendingBlockWithTxs::PendingBlock(block) = current_block else {
            return Err(anyhow!("Expected pending block"));
        };

        Ok(BlockEnv {
            timestamp: block.timestamp,
            number: latest_block.block_number + 1,
            sequencer_address: block.sequencer_address.into(),
            ..Default::default() // set gas prices also
        })
    }

    pub fn start_background_sync(&self) -> Result<()> {
        let client = self.client.clone();
        let sender = self.sender.clone();
        let task_spawner = self.task_spawner.clone();

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
                        let block_env = BlockEnv {
                            number: block.block_number,
                            timestamp: block.timestamp,
                            sequencer_address: block.sequencer_address.into(),
                            ..Default::default() // set gas prices also
                        };

                        sender.send(Some(block_env)).expect("failed to notify");
                    }

                    MaybePendingBlockWithTxs::PendingBlock(block) => {
                        let latest_block = client.block_hash_and_number().await?;
                        let block_env = BlockEnv {
                            timestamp: block.timestamp,
                            number: latest_block.block_number + 1,
                            sequencer_address: block.sequencer_address.into(),
                            ..Default::default() // set gas prices also
                        };

                        sender.send(Some(block_env)).expect("failed to notify");
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
