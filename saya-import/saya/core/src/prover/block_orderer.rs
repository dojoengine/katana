use std::{collections::BTreeMap, marker::PhantomData};

use anyhow::Result;
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::debug;

use crate::{
    prover::{HasBlockNumber, PipelineStage, PipelineStageBuilder},
    service::{Daemon, FinishHandle, ShutdownHandle},
};

/// A pipeline stage that reorders items from concurrent upstream workers into sequential order.
///
/// Workers may complete blocks out of order. `BlockOrderer` buffers items in a `BTreeMap` and
/// emits them strictly in ascending block-number order, starting from `start_block`.
#[derive(Debug)]
pub struct BlockOrderer<T> {
    input_channel: Receiver<T>,
    output_channel: Sender<T>,
    finish_handle: FinishHandle,
    start_block: u64,
}

#[derive(Debug)]
pub struct BlockOrdererBuilder<T> {
    input_channel: Option<Receiver<T>>,
    output_channel: Option<Sender<T>>,
    start_block: Option<u64>,
    _phantom: PhantomData<T>,
}

impl<T> BlockOrdererBuilder<T> {
    pub fn new() -> Self {
        Self {
            input_channel: None,
            output_channel: None,
            start_block: None,
            _phantom: PhantomData,
        }
    }
}

impl<T> Default for BlockOrdererBuilder<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> PipelineStageBuilder for BlockOrdererBuilder<T>
where
    T: HasBlockNumber + Send + 'static,
{
    type Stage = BlockOrderer<T>;

    fn build(self) -> Result<Self::Stage> {
        Ok(BlockOrderer {
            input_channel: self
                .input_channel
                .ok_or_else(|| anyhow::anyhow!("`input_channel` not set"))?,
            output_channel: self
                .output_channel
                .ok_or_else(|| anyhow::anyhow!("`output_channel` not set"))?,
            finish_handle: FinishHandle::new(),
            start_block: self
                .start_block
                .ok_or_else(|| anyhow::anyhow!("`start_block` not set on BlockOrderer"))?,
        })
    }

    fn input_channel(mut self, input_channel: Receiver<T>) -> Self {
        self.input_channel = Some(input_channel);
        self
    }

    fn output_channel(mut self, output_channel: Sender<T>) -> Self {
        self.output_channel = Some(output_channel);
        self
    }

    fn start_block(mut self, start_block: u64) -> Self {
        self.start_block = Some(start_block);
        self
    }
}

impl<T: HasBlockNumber + Send + 'static> PipelineStage for BlockOrderer<T> {
    type Input = T;
    type Output = T;
}

impl<T: HasBlockNumber + Send + 'static> Daemon for BlockOrderer<T> {
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        tokio::spawn(self.run());
    }
}

impl<T: HasBlockNumber + Send + 'static> BlockOrderer<T> {
    async fn run(mut self) {
        let mut pending: BTreeMap<u64, T> = BTreeMap::new();
        let mut next_expected = self.start_block;

        loop {
            // Drain buffered items in order before waiting for more.
            while let Some(item) = pending.remove(&next_expected) {
                if self.output_channel.send(item).await.is_err() {
                    debug!("BlockOrderer output channel closed");
                    self.finish_handle.finish();
                    return;
                }
                next_expected += 1;
            }

            // Wait for the next upstream item.
            let item = tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                item = self.input_channel.recv() => match item {
                    Some(item) => item,
                    None => break,
                },
            };

            pending.insert(item.block_number(), item);
        }

        debug!("BlockOrderer graceful shutdown finished");
        self.finish_handle.finish();
    }
}
