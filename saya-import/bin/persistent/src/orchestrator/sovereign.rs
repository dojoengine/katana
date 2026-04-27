use anyhow::Result;
use swiftness_stark::types::StarkProof;
use tokio::sync::mpsc::Receiver;
use tracing::{debug, info};

use saya_core::{
    block_ingestor::{BlockInfo, BlockIngestor, BlockIngestorBuilder},
    data_availability::{
        DataAvailabilityBackend, DataAvailabilityBackendBuilder, DataAvailabilityCursor,
        DataAvailabilityPointer,
    },
    orchestrator::Genesis,
    prover::{PipelineStage, PipelineStageBuilder, SnosProof},
    service::{Daemon, FinishHandle, ShutdownHandle},
    storage::{BlockWithDa, ChainHead, StorageBackend},
};

/// Size of the `NewBlock` channel.
///
/// Block ingestor implementations would typically always make at least one extra block ready to be
/// sent regardless of whether the channel is full. Therefore, setting this value as `1` should be
/// sufficient.
const BLOCK_INGESTOR_BUFFER_SIZE: usize = 1;

/// Size of the `StarkProof` channel.
const OUTPUT_BUFFER_SIZE: usize = 1;

/// Size of the `DataAvailabilityCursor` channel.
const CURSOR_BUFFER_SIZE: usize = 1;

/// An orchestrator implementation for running a rollup in sovereign mode.
///
/// In this mode, the orchestrator proves blocks and makes full proofs available through a data
/// availability backend. However, no "settlement" is performed in a decentralized manner (e.g. on a
/// base "layer-1" blockchain).
#[derive(Debug)]
pub struct SovereignOrchestrator<I, P, D, S> {
    cursor_channel: Receiver<DataAvailabilityCursor<SnosProof<StarkProof>>>,
    ingestor: I,
    pipeline: P,
    da: D,
    storage: S,
    finish_handle: FinishHandle,
}

#[derive(Debug)]
pub struct SovereignOrchestratorBuilder<I, P, D, S> {
    ingestor_builder: I,
    pipeline_builder: P,
    da_builder: D,
    storage: S,
    genesis: Option<Genesis>,
}

struct SovereignOrchestratorState<S> {
    cursor_channel: Receiver<DataAvailabilityCursor<SnosProof<StarkProof>>>,
    storage: S,
    ingestor_handle: ShutdownHandle,
    pipeline_handle: ShutdownHandle,
    da_handle: ShutdownHandle,
    finish_handle: FinishHandle,
}

impl<I, P, D, S> SovereignOrchestratorBuilder<I, P, D, S> {
    pub fn new(
        ingestor_builder: I,
        pipeline_builder: P,
        da_builder: D,
        storage: S,
        genesis: Option<Genesis>,
    ) -> Self {
        Self {
            ingestor_builder,
            pipeline_builder,
            da_builder,
            storage,
            genesis,
        }
    }
}

impl<I, P, PV, D, DB, S> SovereignOrchestratorBuilder<I, P, D, S>
where
    I: BlockIngestorBuilder + Send,
    P: PipelineStageBuilder<Stage = PV> + Send,
    PV: PipelineStage<Input = BlockInfo, Output = SnosProof<StarkProof>>,
    D: DataAvailabilityBackendBuilder<Backend = DB> + Send,
    DB: DataAvailabilityBackend<Payload = SnosProof<StarkProof>>,
    S: StorageBackend,
{
    pub async fn build(
        self,
    ) -> Result<SovereignOrchestrator<I::Ingestor, P::Stage, D::Backend, S>> {
        let (new_block_tx, new_block_rx) =
            tokio::sync::mpsc::channel::<BlockInfo>(BLOCK_INGESTOR_BUFFER_SIZE);
        let (output_tx, output_rx) =
            tokio::sync::mpsc::channel::<SnosProof<StarkProof>>(OUTPUT_BUFFER_SIZE);
        let (cursor_tx, cursor_rx) = tokio::sync::mpsc::channel::<
            DataAvailabilityCursor<SnosProof<StarkProof>>,
        >(CURSOR_BUFFER_SIZE);

        let chain_head = self.storage.get_chain_head().await;
        let (start_block, da_builder) = match chain_head {
            ChainHead::Genesis => match self.genesis {
                Some(genesis) => (
                    genesis.first_block_number,
                    self.da_builder.last_pointer(None),
                ),
                None => {
                    // In sovereign mode the chain is not settled in a decentralized manner. Without
                    // a pointer to the last published DA we can only rely on the optionally
                    // supplied genesis info for starting the orchestrator.
                    anyhow::bail!("genesis not provided when chain head has not been persisted")
                }
            },
            ChainHead::Block(block_with_da) => (
                block_with_da.height + 1,
                self.da_builder.last_pointer(Some(DataAvailabilityPointer {
                    height: block_with_da.height,
                    commitment: block_with_da.da_pointer.commitment,
                    namespace: block_with_da.da_pointer.namespace,
                })),
            ),
        };

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

        let da = da_builder
            .proof_channel(output_rx)
            .cursor_channel(cursor_tx)
            .build()
            .unwrap();

        Ok(SovereignOrchestrator {
            cursor_channel: cursor_rx,
            ingestor,
            pipeline,
            da,
            storage: self.storage,
            finish_handle: FinishHandle::new(),
        })
    }
}

impl<S> SovereignOrchestratorState<S>
where
    S: StorageBackend,
{
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

            // TODO: error handling
            let da_pointer = new_cursor.pointer.unwrap();

            self.storage
                .set_chain_head(BlockWithDa {
                    height: new_cursor.block_number,
                    da_pointer,
                })
                .await;
            info!(block_number = new_cursor.block_number, "Chain advanced");
        }

        // Request graceful shutdown for all descendant services
        self.ingestor_handle.shutdown();
        self.pipeline_handle.shutdown();
        self.da_handle.shutdown();

        // Wait for all descendant services to finish graceful shutdown
        futures_util::future::join_all([
            self.ingestor_handle.finished(),
            self.pipeline_handle.finished(),
            self.da_handle.finished(),
        ])
        .await;

        debug!("Graceful shutdown finished");
        self.finish_handle.finish();
    }
}

impl<I, P, D, S> Daemon for SovereignOrchestrator<I, P, D, S>
where
    I: BlockIngestor + Send,
    P: PipelineStage + Send,
    D: DataAvailabilityBackend + Send,
    S: StorageBackend + Send + 'static,
{
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        let state = SovereignOrchestratorState {
            cursor_channel: self.cursor_channel,
            storage: self.storage,
            ingestor_handle: self.ingestor.shutdown_handle(),
            pipeline_handle: self.pipeline.shutdown_handle(),
            da_handle: self.da.shutdown_handle(),
            finish_handle: self.finish_handle,
        };

        self.ingestor.start();
        self.pipeline.start();
        self.da.start();

        tokio::spawn(state.run());
    }
}
