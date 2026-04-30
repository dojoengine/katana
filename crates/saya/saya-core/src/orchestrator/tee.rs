//! Full TEE orchestrator — wires together all TEE pipeline stages.
//!
//! Pipeline:
//!
//! ```text
//!   BlockIngestor          (sequential; emits ordered Vec<BlockInfo> batches)
//!       │ Vec<BlockInfo>
//!   TeeAttestor            (fetches TEE attestation from Katana for the batch)
//!       │ TeeAttestation
//!   TeeProver              (generates SP1 Groth16 proof, carries block state fields)
//!       │ TeeProof
//!   TeeSettlementBackend   (builds PiltoverInput::TeeInput, submits update_state)
//!       │ SettlementCursor
//!   TeeOrchestrator loop   (logs confirmed blocks)
//! ```

use anyhow::Result;
use tokio::sync::mpsc::Receiver;
use tracing::{debug, info};

use crate::{
    block_ingestor::{BatchingBlockIngestorBuilder, BlockInfo, BlockIngestor},
    prover::{tee::TeeProof, PipelineStage, PipelineStageBuilder},
    service::{Daemon, FinishHandle, ShutdownHandle},
    settlement::{SettlementBackend, SettlementCursor, TeeSettlementBackendBuilder},
    tee::TeeAttestation,
};

const BLOCK_INGESTOR_BUFFER_SIZE: usize = 4;
const ATTESTATION_BUFFER_SIZE: usize = 4;
const PROOF_BUFFER_SIZE: usize = 4;
const SETTLE_CURSOR_BUFFER_SIZE: usize = 4;

/// An orchestrator for running a rollup in TEE mode with external proof generation.
#[derive(Debug)]
pub struct TeeOrchestrator<I, A, P, S> {
    cursor_channel: Receiver<SettlementCursor>,
    ingestor: I,
    attestor: A,
    prover: P,
    settlement: S,
    finish_handle: FinishHandle,
}

#[derive(Debug)]
pub struct TeeOrchestratorBuilder<I, A, P, S> {
    ingestor_builder: I,
    attestor_builder: A,
    prover_builder: P,
    settlement_builder: S,
}

struct TeeOrchestratorState {
    cursor_channel: Receiver<SettlementCursor>,
    ingestor_handle: ShutdownHandle,
    attestor_handle: ShutdownHandle,
    prover_handle: ShutdownHandle,
    settlement_handle: ShutdownHandle,
    finish_handle: FinishHandle,
}

impl<I, A, P, S> TeeOrchestratorBuilder<I, A, P, S> {
    pub fn new(
        ingestor_builder: I,
        attestor_builder: A,
        prover_builder: P,
        settlement_builder: S,
    ) -> Self {
        Self {
            ingestor_builder,
            attestor_builder,
            prover_builder,
            settlement_builder,
        }
    }
}

impl<I, A, AV, P, PV, S> TeeOrchestratorBuilder<I, A, P, S>
where
    I: BatchingBlockIngestorBuilder + Send,
    A: PipelineStageBuilder<Stage = AV> + Send,
    AV: PipelineStage<Input = Vec<BlockInfo>, Output = TeeAttestation>,
    P: PipelineStageBuilder<Stage = PV> + Send,
    PV: PipelineStage<Input = TeeAttestation, Output = TeeProof>,
    S: TeeSettlementBackendBuilder + Send,
{
    pub async fn build(self) -> Result<TeeOrchestrator<I::Ingestor, AV, PV, S::Backend>> {
        let (new_block_tx, new_block_rx) =
            tokio::sync::mpsc::channel::<Vec<BlockInfo>>(BLOCK_INGESTOR_BUFFER_SIZE);
        let (attestation_tx, attestation_rx) =
            tokio::sync::mpsc::channel::<TeeAttestation>(ATTESTATION_BUFFER_SIZE);
        let (proof_tx, proof_rx) = tokio::sync::mpsc::channel::<TeeProof>(PROOF_BUFFER_SIZE);
        let (settle_cursor_tx, settle_cursor_rx) =
            tokio::sync::mpsc::channel::<SettlementCursor>(SETTLE_CURSOR_BUFFER_SIZE);

        let settlement = self
            .settlement_builder
            .proof_channel(proof_rx)
            .cursor_channel(settle_cursor_tx)
            .build()
            .await?;

        let start_block = settlement.get_block_number().await? + 1;
        let start_block: u64 = start_block.try_into()?;

        let ingestor = self
            .ingestor_builder
            .start_block(start_block)
            .channel(new_block_tx)
            .build()?;

        let attestor = self
            .attestor_builder
            .input_channel(new_block_rx)
            .output_channel(attestation_tx)
            .build()?;

        let prover = self
            .prover_builder
            .input_channel(attestation_rx)
            .output_channel(proof_tx)
            .build()?;

        Ok(TeeOrchestrator {
            cursor_channel: settle_cursor_rx,
            ingestor,
            attestor,
            prover,
            settlement,
            finish_handle: FinishHandle::new(),
        })
    }
}

impl TeeOrchestratorState {
    async fn run(mut self) {
        loop {
            let new_cursor = tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                new_cursor = self.cursor_channel.recv() => new_cursor,
            };

            let new_cursor = new_cursor.unwrap();

            info!(
                block_number = new_cursor.block_number,
                transaction_hash = %format!("{:#064x}", new_cursor.transaction_hash),
                "Chain advanced to new block"
            );
        }

        self.ingestor_handle.shutdown();
        self.attestor_handle.shutdown();
        self.prover_handle.shutdown();
        self.settlement_handle.shutdown();

        futures_util::future::join_all([
            self.ingestor_handle.finished(),
            self.attestor_handle.finished(),
            self.prover_handle.finished(),
            self.settlement_handle.finished(),
        ])
        .await;

        debug!("Graceful shutdown finished");
        self.finish_handle.finish();
    }
}

impl<I, A, P, S> Daemon for TeeOrchestrator<I, A, P, S>
where
    I: BlockIngestor + Send,
    A: PipelineStage + Send,
    P: PipelineStage + Send,
    S: SettlementBackend + Send,
{
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        let state = TeeOrchestratorState {
            cursor_channel: self.cursor_channel,
            ingestor_handle: self.ingestor.shutdown_handle(),
            attestor_handle: self.attestor.shutdown_handle(),
            prover_handle: self.prover.shutdown_handle(),
            settlement_handle: self.settlement.shutdown_handle(),
            finish_handle: self.finish_handle,
        };

        self.ingestor.start();
        self.attestor.start();
        self.prover.start();
        self.settlement.start();

        tokio::spawn(state.run());
    }
}
