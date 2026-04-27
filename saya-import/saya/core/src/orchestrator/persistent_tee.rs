use anyhow::Result;
use tokio::sync::mpsc::Receiver;
use tracing::{debug, info};

use crate::{
    block_ingestor::{BlockInfo, BlockIngestor, BlockIngestorBuilder},
    data_availability::DataAvailabilityCursor,
    prover::{BlockOrderer, BlockOrdererBuilder, PipelineStageBuilder},
    service::{Daemon, FinishHandle, ShutdownHandle},
    settlement::{SettlementBackend, SettlementBackendBuilder, SettlementCursor},
};

/// Size of the `BlockInfo` channel between ingestor and the orderer.
const BLOCK_INGESTOR_BUFFER_SIZE: usize = 4;

/// Size of the `BlockInfo` channel between orderer and the bridge task.
const ORDERER_BUFFER_SIZE: usize = 4;

/// Size of the `DataAvailabilityCursor` channel fed into settlement.
const DA_CURSOR_BUFFER_SIZE: usize = 4;

/// Size of the `SettlementCursor` channel.
const SETTLE_CURSOR_BUFFER_SIZE: usize = 4;

/// An orchestrator for running a rollup in TEE (Trusted Execution Environment) persistent mode.
///
/// In this mode the block is proved inside a secure enclave, so no external proving service or
/// data availability layer is required. The orchestrator therefore only wires together two
/// components:
///
/// 1. A **block ingestor** — polls the rollup RPC for new blocks and generates the Cairo PIE.
/// 2. A **settlement backend** — submits the state-root transition on the base layer.
///
/// The ingestor output (`BlockInfo`) is forwarded to the settlement backend through an internal
/// adapter task that wraps each item into a `DataAvailabilityCursor` with no DA pointer, keeping
/// the settlement trait interface unchanged.
#[derive(Debug)]
pub struct PersistentTeeOrchestrator<I, S> {
    cursor_channel: Receiver<SettlementCursor>,
    ingestor: I,
    orderer: BlockOrderer<BlockInfo>,
    settlement: S,
    finish_handle: FinishHandle,
}

#[derive(Debug)]
pub struct PersistentTeeOrchestratorBuilder<I, S> {
    ingestor_builder: I,
    settlement_builder: S,
}

struct PersistentTeeOrchestratorState {
    cursor_channel: Receiver<SettlementCursor>,
    ingestor_handle: ShutdownHandle,
    orderer_handle: ShutdownHandle,
    settlement_handle: ShutdownHandle,
    finish_handle: FinishHandle,
}

impl<I, S> PersistentTeeOrchestratorBuilder<I, S> {
    pub fn new(ingestor_builder: I, settlement_builder: S) -> Self {
        Self {
            ingestor_builder,
            settlement_builder,
        }
    }
}

impl<I, S> PersistentTeeOrchestratorBuilder<I, S>
where
    I: BlockIngestorBuilder + Send,
    S: SettlementBackendBuilder + Send,
{
    pub async fn build(self) -> Result<PersistentTeeOrchestrator<I::Ingestor, S::Backend>> {
        let (new_block_tx, new_block_rx) =
            tokio::sync::mpsc::channel::<BlockInfo>(BLOCK_INGESTOR_BUFFER_SIZE);
        let (ordered_tx, mut ordered_rx) =
            tokio::sync::mpsc::channel::<BlockInfo>(ORDERER_BUFFER_SIZE);
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

        // Since the `Felt` type is wrapping (`Felt::MAX + 1 = 0`), there is no
        // need for a special case for the genesis block, and `+1` works as expected.
        let start_block = settlement.get_block_number().await? + 1;
        let start_block: u64 = start_block.try_into()?;

        let ingestor = self
            .ingestor_builder
            .start_block(start_block)
            .channel(new_block_tx)
            .build()
            .unwrap();

        let orderer = BlockOrdererBuilder::new()
            .start_block(start_block)
            .input_channel(new_block_rx)
            .output_channel(ordered_tx)
            .build()?;

        // Adapter task: wraps each in-order `BlockInfo` in a `DataAvailabilityCursor` with no DA
        // pointer so the settlement backend can remain unaware of the TEE-specific flow.
        tokio::spawn(async move {
            while let Some(block_info) = ordered_rx.recv().await {
                let cursor = DataAvailabilityCursor {
                    block_number: block_info.number,
                    pointer: None,
                    full_payload: block_info,
                };
                if da_cursor_tx.send(cursor).await.is_err() {
                    break;
                }
            }
        });

        Ok(PersistentTeeOrchestrator {
            cursor_channel: settle_cursor_rx,
            ingestor,
            orderer,
            settlement,
            finish_handle: FinishHandle::new(),
        })
    }
}

impl PersistentTeeOrchestratorState {
    async fn run(mut self) {
        loop {
            // TODO: handle unexpected exit of descendant services
            let new_cursor = tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                new_cursor = self.cursor_channel.recv() => new_cursor,
            };

            // The settlement backend won't drop the sender while running.
            let new_cursor = new_cursor.unwrap();

            info!(
                block_number = new_cursor.block_number,
                transaction_hash = %format!("{:#064x}", new_cursor.transaction_hash),
                "Chain advanced to new block"
            );
        }

        // Request graceful shutdown for all descendant services.
        self.ingestor_handle.shutdown();
        self.orderer_handle.shutdown();
        self.settlement_handle.shutdown();

        // Wait for all descendant services to finish.
        futures_util::future::join_all([
            self.ingestor_handle.finished(),
            self.orderer_handle.finished(),
            self.settlement_handle.finished(),
        ])
        .await;

        debug!("Graceful shutdown finished");
        self.finish_handle.finish();
    }
}

impl<I, S> Daemon for PersistentTeeOrchestrator<I, S>
where
    I: BlockIngestor + Send,
    S: SettlementBackend + Send,
{
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        let state = PersistentTeeOrchestratorState {
            cursor_channel: self.cursor_channel,
            ingestor_handle: self.ingestor.shutdown_handle(),
            orderer_handle: self.orderer.shutdown_handle(),
            settlement_handle: self.settlement.shutdown_handle(),
            finish_handle: self.finish_handle,
        };

        self.ingestor.start();
        self.orderer.start();
        self.settlement.start();

        tokio::spawn(state.run());
    }
}
