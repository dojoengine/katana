use anyhow::Result;
use serde::{Deserialize, Serialize};
use starknet::core::types::Felt;
use starknet::core::types::StateUpdate;
#[cfg(feature = "snos")]
use swiftness_stark::types::StarkProof;
use tokio::sync::mpsc::{Receiver, Sender};

mod celestia;
pub use celestia::{CelestiaDataAvailabilityBackend, CelestiaDataAvailabilityBackendBuilder};

mod noop;
pub use noop::{NoopDataAvailabilityBackend, NoopDataAvailabilityBackendBuilder};

#[cfg(feature = "snos")]
use crate::prover::SnosProof;
use crate::service::Daemon;

pub trait DataAvailabilityBackendBuilder {
    type Backend: DataAvailabilityBackend;

    fn build(self) -> Result<Self::Backend>;

    fn last_pointer(self, last_pointer: Option<DataAvailabilityPointer>) -> Self;

    fn proof_channel(
        self,
        proof_channel: Receiver<<Self::Backend as DataAvailabilityBackend>::Payload>,
    ) -> Self;

    fn cursor_channel(
        self,
        cursor_channel: Sender<
            DataAvailabilityCursor<<Self::Backend as DataAvailabilityBackend>::Payload>,
        >,
    ) -> Self;
}

pub trait DataAvailabilityBackend: Daemon {
    type Payload: DataAvailabilityPayload;
}

pub trait DataAvailabilityPayload: Clone + Send {
    type Packet: Serialize + Send;

    fn block_number(&self) -> u64;

    fn into_packet(self, ctx: DataAvailabilityPacketContext) -> Self::Packet;
}

pub struct DataAvailabilityPacketContext {
    pub prev: Option<DataAvailabilityPointer>,
}

/// Data made available in `sovereign` mode, which contains the full `snos` proof to be verified
/// off-chain.
#[cfg(feature = "snos")]
#[derive(Debug, Serialize, Deserialize)]
pub struct SovereignPacket {
    /// Pointer to the previous [`SovereignPacket`].
    pub prev: Option<DataAvailabilityPointer>,
    /// The content of the packet.
    pub proof: SnosProof<StarkProof>,
}

/// Data made available in `persistent` mode, containing only the `snos` output.
#[derive(Debug, Serialize, Deserialize)]
pub struct PersistentPacket {
    pub state_update: Option<StateUpdate>,
}

// TODO: abstract over this to allow other DA backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataAvailabilityPointer {
    /// Celestia block height.
    pub height: u64,
    /// Celestia blob commitment.
    pub commitment: [u8; 32],
    /// Celestia namespace ID.
    pub namespace: Felt,
}

// TODO: abstract over this to allow other DA backends.
#[derive(Debug, Clone)]
pub struct DataAvailabilityCursor<P> {
    /// State transition block.
    pub block_number: u64,
    /// Pointer to location of data availability. `None` if data publishing was not performed.
    pub pointer: Option<DataAvailabilityPointer>,
    /// Full content of the payload.
    pub full_payload: P,
}
