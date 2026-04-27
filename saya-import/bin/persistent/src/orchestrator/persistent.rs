use anyhow::Result;
use tokio::sync::mpsc::Receiver;
use tracing::{debug, info};

use saya_core::{
    block_ingestor::{BlockInfo, BlockIngestor, BlockIngestorBuilder},
    data_availability::{
        DataAvailabilityBackend, DataAvailabilityBackendBuilder, DataAvailabilityCursor,
    },
    prover::{PipelineStage, PipelineStageBuilder},
    service::{Daemon, FinishHandle, ShutdownHandle},
    settlement::{SettlementBackend, SettlementBackendBuilder, SettlementCursor},
};

/// Size of the `NewBlock` channel.
///
/// Block ingestor implementations would typically always make at least one extra block ready to be
/// sent regardless of whether the channel is full. Therefore, setting this value as `1` should be
/// sufficient.
const BLOCK_INGESTOR_BUFFER_SIZE: usize = 4;

/// Size of the `StarkProof` channel.
const OUTPUT_BUFFER_SIZE: usize = 4;

/// Size of the `DataAvailabilityCursor` channel.
const DA_CURSOR_BUFFER_SIZE: usize = 4;

/// Size of the `SettlementCursor` channel.
const SETTLE_CURSOR_BUFFER_SIZE: usize = 4;

/// An orchestrator implementation for running a rollup in persistent mode.
///
/// In this mode, the orchestrator proves blocks and makes full proofs available through a data
/// availability backend. It then applies the state root transition on a settlement layer and
/// publishes the data availability fact simultaneously.
///
/// Notably, the data availability fact is not verified and opaque to the settlement layer.
/// Therefore, with the current implementation, there's a risk that a rollup's sequencer would
/// withhold full state transition data, making it impossible to access the latest state.
#[derive(Debug)]
pub struct PersistentOrchestrator<I, P, D, S> {
    cursor_channel: Receiver<SettlementCursor>,
    ingestor: I,
    pipeline: P,
    da: D,
    settlement: S,
    finish_handle: FinishHandle,
}

#[derive(Debug)]
pub struct PersistentOrchestratorBuilder<I, P, D, S> {
    ingestor_builder: I,
    pipeline_builder: P,
    da_builder: D,
    settlement_builder: S,
}

struct PersistentOrchestratorState {
    cursor_channel: Receiver<SettlementCursor>,
    ingestor_handle: ShutdownHandle,
    pipeline_handle: ShutdownHandle,
    da_handle: ShutdownHandle,
    settlement_handle: ShutdownHandle,
    finish_handle: FinishHandle,
}

impl<I, P, D, S> PersistentOrchestratorBuilder<I, P, D, S> {
    pub fn new(
        ingestor_builder: I,
        pipeline_builder: P,
        da_builder: D,
        settlement_builder: S,
    ) -> Self {
        Self {
            ingestor_builder,
            pipeline_builder,
            da_builder,
            settlement_builder,
        }
    }
}

impl<I, P, PV, D, DB, S> PersistentOrchestratorBuilder<I, P, D, S>
where
    I: BlockIngestorBuilder + Send,
    P: PipelineStageBuilder<Stage = PV> + Send,
    PV: PipelineStage<Input = BlockInfo, Output = BlockInfo>,
    D: DataAvailabilityBackendBuilder<Backend = DB> + Send,
    DB: DataAvailabilityBackend<Payload = BlockInfo>,
    S: SettlementBackendBuilder + Send,
{
    pub async fn build(
        self,
    ) -> Result<PersistentOrchestrator<I::Ingestor, P::Stage, D::Backend, S::Backend>> {
        let (new_block_tx, new_block_rx) =
            tokio::sync::mpsc::channel::<BlockInfo>(BLOCK_INGESTOR_BUFFER_SIZE);
        let (output_tx, output_rx) = tokio::sync::mpsc::channel::<BlockInfo>(OUTPUT_BUFFER_SIZE);
        let (da_cursor_tx, da_cursor_rx) =
            tokio::sync::mpsc::channel::<DataAvailabilityCursor<BlockInfo>>(DA_CURSOR_BUFFER_SIZE);
        let (settle_cursor_tx, settle_cursor_rx) =
            tokio::sync::mpsc::channel::<SettlementCursor>(SETTLE_CURSOR_BUFFER_SIZE);

        let settlement = self
            .settlement_builder
            .da_channel(da_cursor_rx)
            .cursor_channel(settle_cursor_tx)
            .build()
            .await
            .unwrap();

        // Since the `Felt` type is wrapping (`Felt::MAX + 1 = 0`), there is not
        // need for a special case for the genesis block, and `+1` works as expected.
        //
        // TODO: should we change to `settlement.next_block_number()` instead to always return `u64`?
        let start_block = settlement.get_block_number().await? + 1;

        // Now that the special value of `Felt::MAX` is handled, we can use the block number as `u64`.
        let start_block: u64 = start_block.try_into()?;

        let ingestor = self
            .ingestor_builder
            .start_block(start_block)
            .channel(new_block_tx)
            .build()
            .unwrap();

        let pipeline = self
            .pipeline_builder
            .start_block(start_block)
            .input_channel(new_block_rx)
            .output_channel(output_tx)
            .build()
            .unwrap();

        let da = self
            .da_builder
            .proof_channel(output_rx)
            .cursor_channel(da_cursor_tx)
            .build()
            .unwrap();

        Ok(PersistentOrchestrator {
            cursor_channel: settle_cursor_rx,
            ingestor,
            pipeline,
            da,
            settlement,
            finish_handle: FinishHandle::new(),
        })
    }
}

impl PersistentOrchestratorState {
    async fn run(mut self) {
        loop {
            // TODO: handle unexpected exit of descendant services
            let new_cursor = tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                new_cursor = self.cursor_channel.recv() => new_cursor,
            };

            // This should be fine for now as da backends wouldn't drop senders. This might change
            // in the future.
            let new_cursor = new_cursor.unwrap();

            info!(
                block_number = new_cursor.block_number,
                transaction_hash = %format!("{:#064x}", new_cursor.transaction_hash),
                "Chain advanced to new block"
            );
        }

        // Request graceful shutdown for all descendant services
        self.ingestor_handle.shutdown();
        self.pipeline_handle.shutdown();
        self.da_handle.shutdown();
        self.settlement_handle.shutdown();

        // Wait for all descendant services to finish graceful shutdown
        futures_util::future::join_all([
            self.ingestor_handle.finished(),
            self.pipeline_handle.finished(),
            self.da_handle.finished(),
            self.settlement_handle.finished(),
        ])
        .await;

        debug!("Graceful shutdown finished");
        self.finish_handle.finish();
    }
}

impl<I, P, D, S> Daemon for PersistentOrchestrator<I, P, D, S>
where
    I: BlockIngestor + Send,
    P: PipelineStage + Send,
    D: DataAvailabilityBackend + Send,
    S: SettlementBackend + Send,
{
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        let state = PersistentOrchestratorState {
            cursor_channel: self.cursor_channel,
            ingestor_handle: self.ingestor.shutdown_handle(),
            pipeline_handle: self.pipeline.shutdown_handle(),
            da_handle: self.da.shutdown_handle(),
            settlement_handle: self.settlement.shutdown_handle(),
            finish_handle: self.finish_handle,
        };

        self.ingestor.start();
        self.pipeline.start();
        self.da.start();
        self.settlement.start();

        tokio::spawn(state.run());
    }
}
