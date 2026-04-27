use anyhow::Result;

use starknet::core::types::StateUpdate;
use tokio::sync::mpsc::Sender;

mod polling;

pub use polling::{
    BatchingPollingBlockIngestor, BatchingPollingBlockIngestorBuilder, PollingBlockIngestor,
    PollingBlockIngestorBuilder,
};

use crate::{service::Daemon, storage::BlockStatus};

pub trait BlockIngestorBuilder {
    type Ingestor: BlockIngestor;

    fn build(self) -> Result<Self::Ingestor>;

    fn start_block(self, start_block: u64) -> Self;

    fn channel(self, channel: Sender<BlockInfo>) -> Self;
}

/// Like [`BlockIngestorBuilder`] but emits ordered *batches* of blocks downstream.
///
/// Used by the TEE pipeline where a single attestation covers a whole batch.
pub trait BatchingBlockIngestorBuilder {
    type Ingestor: BlockIngestor;

    fn build(self) -> Result<Self::Ingestor>;

    fn start_block(self, start_block: u64) -> Self;

    fn channel(self, channel: Sender<Vec<BlockInfo>>) -> Self;
}

pub trait BlockIngestor: Daemon {}

#[derive(Debug, Clone)]
pub struct BlockInfo {
    pub number: u64,
    pub status: BlockStatus,
    pub state_update: Option<StateUpdate>,
}
