use anyhow::Result;
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::debug;

use crate::{
    prover::{PipelineStage, PipelineStageBuilder},
    service::{Daemon, FinishHandle, ShutdownHandle},
};

const BRIDGE_BUFFER_SIZE: usize = 4;

#[derive(Debug)]
pub struct PipelineChain<U, D> {
    upstream: U,
    downstream: D,
    finish_handle: FinishHandle,
}

#[derive(Debug)]
pub struct PipelineChainBuilder<U, D> {
    upstream_builder: U,
    downstream_builder: D,
}

struct PipelineChainState {
    upstream_handle: ShutdownHandle,
    downstream_handle: ShutdownHandle,
    finish_handle: FinishHandle,
}

impl<U, D> PipelineChainBuilder<U, D> {
    pub fn new(upstream_builder: U, downstream_builder: D) -> Self {
        Self {
            upstream_builder,
            downstream_builder,
        }
    }
}

impl<U, D, UV, DV, I> PipelineStageBuilder for PipelineChainBuilder<U, D>
where
    U: PipelineStageBuilder<Stage = UV>,
    D: PipelineStageBuilder<Stage = DV>,
    UV: PipelineStage<Output = I>,
    DV: PipelineStage<Input = I>,
{
    type Stage = PipelineChain<U::Stage, D::Stage>;

    fn build(self) -> Result<Self::Stage> {
        let (bridge_tx, bridge_rx) = tokio::sync::mpsc::channel::<I>(BRIDGE_BUFFER_SIZE);

        Ok(PipelineChain {
            upstream: self.upstream_builder.output_channel(bridge_tx).build()?,
            downstream: self.downstream_builder.input_channel(bridge_rx).build()?,
            finish_handle: FinishHandle::new(),
        })
    }

    fn input_channel(self, block_channel: Receiver<<Self::Stage as PipelineStage>::Input>) -> Self {
        Self {
            upstream_builder: self.upstream_builder.input_channel(block_channel),
            downstream_builder: self.downstream_builder,
        }
    }

    fn output_channel(
        self,
        output_channel: Sender<<Self::Stage as PipelineStage>::Output>,
    ) -> Self {
        Self {
            upstream_builder: self.upstream_builder,
            downstream_builder: self.downstream_builder.output_channel(output_channel),
        }
    }

    fn start_block(self, start_block: u64) -> Self {
        Self {
            upstream_builder: self.upstream_builder.start_block(start_block),
            downstream_builder: self.downstream_builder.start_block(start_block),
        }
    }
}

impl PipelineChainState {
    async fn run(self) {
        self.finish_handle.shutdown_requested().await;

        // Request graceful shutdown for all descendant services
        self.upstream_handle.shutdown();
        self.downstream_handle.shutdown();

        // Wait for all descendant services to finish graceful shutdown
        futures_util::future::join_all([
            self.upstream_handle.finished(),
            self.downstream_handle.finished(),
        ])
        .await;

        debug!("Graceful shutdown finished");
        self.finish_handle.finish();
    }
}

impl<U, D, I> PipelineStage for PipelineChain<U, D>
where
    U: PipelineStage<Output = I>,
    D: PipelineStage<Input = I>,
{
    type Input = U::Input;
    type Output = D::Output;
}

impl<U, D, I> Daemon for PipelineChain<U, D>
where
    U: PipelineStage<Output = I>,
    D: PipelineStage<Input = I>,
{
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        let state = PipelineChainState {
            upstream_handle: self.upstream.shutdown_handle(),
            downstream_handle: self.downstream.shutdown_handle(),
            finish_handle: self.finish_handle,
        };

        self.upstream.start();
        self.downstream.start();

        tokio::spawn(state.run());
    }
}
