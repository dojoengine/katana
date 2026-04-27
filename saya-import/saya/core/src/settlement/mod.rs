use std::future::Future;

use anyhow::Result;
use starknet_types_core::felt::Felt;
use tokio::sync::mpsc::{Receiver, Sender};

use crate::{
    block_ingestor::BlockInfo, data_availability::DataAvailabilityCursor, prover::TeeProof,
    service::Daemon,
};

pub trait SettlementBackendBuilder {
    type Backend: SettlementBackend;

    fn build(self) -> impl Future<Output = Result<Self::Backend>> + Send;

    fn da_channel(self, da_channel: Receiver<DataAvailabilityCursor<BlockInfo>>) -> Self;

    fn cursor_channel(self, cursor_channel: Sender<SettlementCursor>) -> Self;
}

pub trait SettlementBackend: Daemon {
    /// Gets the block number of the last block verified by the settlement layer.
    ///
    /// It returns a `Felt` since the previous block value for genesis block is `Felt::MAX`.
    fn get_block_number(&self) -> impl Future<Output = Result<Felt>> + Send;
}

/// Settlement backend builder for the TEE pipeline.
///
/// Receives [`TeeProof`] directly — all calldata fields are carried on the proof so no
/// `DataAvailabilityCursor` adapter is needed.
pub trait TeeSettlementBackendBuilder {
    type Backend: SettlementBackend;

    fn build(self) -> impl Future<Output = Result<Self::Backend>> + Send;

    fn proof_channel(self, proof_channel: Receiver<TeeProof>) -> Self;

    fn cursor_channel(self, cursor_channel: Sender<SettlementCursor>) -> Self;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettlementCursor {
    /// Number/height of the rollup block that's been settled.
    ///
    /// This does NOT refer to the settlement layer block where the transaction is included.
    pub block_number: u64,
    /// Settlement transaction hash.
    pub transaction_hash: Felt,
}
