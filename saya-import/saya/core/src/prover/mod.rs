use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{Receiver, Sender};

use crate::{
    block_ingestor::BlockInfo,
    data_availability::{DataAvailabilityPacketContext, DataAvailabilityPayload, PersistentPacket},
    service::Daemon,
};

mod recursive;
pub use recursive::{PipelineChain, PipelineChainBuilder};

mod block_orderer;
pub use block_orderer::{BlockOrderer, BlockOrdererBuilder};

pub mod tee;
pub use tee::{TeeProof, TeeProver, TeeProverBuilder};

#[cfg(feature = "snos")]
use crate::data_availability::SovereignPacket;
#[cfg(feature = "snos")]
use swiftness_stark::types::StarkProof;

/// Implemented by pipeline items that carry a block number.
pub trait HasBlockNumber {
    fn block_number(&self) -> u64;
}

impl HasBlockNumber for BlockInfo {
    fn block_number(&self) -> u64 {
        self.number
    }
}

impl<P> HasBlockNumber for SnosProof<P> {
    fn block_number(&self) -> u64 {
        self.block_number
    }
}

pub trait PipelineStageBuilder {
    type Stage: PipelineStage;

    fn build(self) -> Result<Self::Stage>;

    fn input_channel(self, block_channel: Receiver<<Self::Stage as PipelineStage>::Input>) -> Self;

    fn output_channel(self, output_channel: Sender<<Self::Stage as PipelineStage>::Output>)
        -> Self;

    /// Propagate the starting block number to interested stages (e.g. `BlockOrderer`).
    ///
    /// The default implementation is a no-op; stages that need `start_block` should override it.
    fn start_block(self, _start_block: u64) -> Self
    where
        Self: Sized,
    {
        self
    }
}

pub trait PipelineStage: Daemon {
    type Input: Send + 'static;
    type Output: Send + 'static;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnosProof<P> {
    pub block_number: u64,
    pub proof: P,
}

#[cfg(feature = "snos")]
impl DataAvailabilityPayload for SnosProof<StarkProof> {
    type Packet = SovereignPacket;

    fn block_number(&self) -> u64 {
        self.block_number
    }

    fn into_packet(self, ctx: DataAvailabilityPacketContext) -> Self::Packet {
        SovereignPacket {
            prev: ctx.prev,
            proof: self,
        }
    }
}

impl DataAvailabilityPayload for BlockInfo {
    type Packet = PersistentPacket;

    fn block_number(&self) -> u64 {
        self.number
    }

    fn into_packet(self, _ctx: DataAvailabilityPacketContext) -> Self::Packet {
        PersistentPacket {
            state_update: self.state_update,
        }
    }
}
