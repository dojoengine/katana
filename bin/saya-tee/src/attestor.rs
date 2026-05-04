//! TEE attestor — fetches attestation data from the Katana rollup node.
//!
//! The attestor receives an ordered batch of [`BlockInfo`] from the upstream ingestor and
//! fetches a single TEE attestation covering the entire batch from the Katana JSON-RPC.

use std::time::Duration;

use anyhow::Result;
use katana_rpc_api::tee::TeeApiClient;
use katana_rpc_api::HttpClientBuilder;
use saya_core::block_ingestor::BlockInfo;
#[allow(unused_imports)]
use saya_core::prover::HasBlockNumber;
use saya_core::prover::{PipelineStage, PipelineStageBuilder};
use saya_core::service::{Daemon, FinishHandle, ShutdownHandle};
use saya_core::tee::TeeAttestation;
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::{debug, error, info};
use url::Url;

/// Fetches TEE attestation from the Katana rollup node for each incoming batch of
/// [`BlockInfo`].
#[derive(Debug)]
pub struct TeeAttestor {
    katana_rpc: Url,
    _poll_interval: Duration,
    input_channel: Receiver<Vec<BlockInfo>>,
    output_channel: Sender<TeeAttestation>,
    finish_handle: FinishHandle,
}

#[derive(Debug)]
pub struct TeeAttestorBuilder {
    katana_rpc: Url,
    poll_interval: Duration,
    input_channel: Option<Receiver<Vec<BlockInfo>>>,
    output_channel: Option<Sender<TeeAttestation>>,
}

impl TeeAttestorBuilder {
    pub fn new(katana_rpc: Url, poll_interval: Duration) -> Self {
        Self { katana_rpc, poll_interval, input_channel: None, output_channel: None }
    }
}

impl PipelineStageBuilder for TeeAttestorBuilder {
    type Stage = TeeAttestor;

    fn build(self) -> Result<Self::Stage> {
        Ok(TeeAttestor {
            katana_rpc: self.katana_rpc,
            _poll_interval: self.poll_interval,
            input_channel: self
                .input_channel
                .ok_or_else(|| anyhow::anyhow!("`input_channel` not set"))?,
            output_channel: self
                .output_channel
                .ok_or_else(|| anyhow::anyhow!("`output_channel` not set"))?,
            finish_handle: FinishHandle::new(),
        })
    }

    fn input_channel(mut self, input_channel: Receiver<Vec<BlockInfo>>) -> Self {
        self.input_channel = Some(input_channel);
        self
    }

    fn output_channel(mut self, output_channel: Sender<TeeAttestation>) -> Self {
        self.output_channel = Some(output_channel);
        self
    }
}

impl PipelineStage for TeeAttestor {
    type Input = Vec<BlockInfo>;
    type Output = TeeAttestation;
}

impl Daemon for TeeAttestor {
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        tokio::spawn(self.run());
    }
}

impl TeeAttestor {
    async fn run(mut self) {
        loop {
            let blocks = tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                batch = self.input_channel.recv() => match batch {
                    Some(b) => b,
                    None => break,
                },
            };
            info!(
                first_block = ?blocks.first().map(|b| b.number),
                last_block = ?blocks.last().map(|b| b.number),
                count = blocks.len(),
                "Fetching TEE attestation for block batch"
            );

            let attestation = match self.fetch_attestation(blocks).await {
                Ok(a) => a,
                Err(e) => {
                    error!("Failed to fetch TEE attestation: {}", e);
                    continue;
                }
            };

            info!("Fetched TEE attestation for block batch, sending to next stage");
            tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                _ = self.output_channel.send(attestation) => {},
            }
        }

        debug!("TeeAttestor graceful shutdown finished");
        self.finish_handle.finish();
    }

    async fn fetch_attestation(&self, blocks: Vec<BlockInfo>) -> Result<TeeAttestation> {
        let rpc_client = HttpClientBuilder::new().build(self.katana_rpc.clone())?;

        let block_number = blocks.last().expect("non-empty batch").number;
        let prev_block = blocks.first().expect("non-empty batch").number;
        let prev_block = if prev_block == 0 { None } else { Some(prev_block.saturating_sub(1)) };

        let attestation = rpc_client.generate_quote(prev_block, block_number).await?;

        Ok(TeeAttestation { blocks, attestation })
    }
}
