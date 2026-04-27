use std::sync::Arc;

use anyhow::Result;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use generate_pie::{
    generate_pie,
    types::{ChainConfig, OsHintsConfiguration},
};

use crate::atlantic::compress_pie;
use saya_core::{
    block_ingestor::BlockInfo,
    prover::{PipelineStage, PipelineStageBuilder},
    service::{Daemon, FinishHandle, ShutdownHandle},
    storage::{BlockStatus, PersistantStorage, Step},
};
use starknet_api::{contract_address, core::ChainId};
use tokio::{
    sync::{
        mpsc::{Receiver, Sender},
        Mutex,
    },
    task,
};
use tracing::{debug, error, info, trace};
use url::Url;

const KATANA_DEFAULT_TOKEN_ADDRESS: &str =
    "0x2e7442625bab778683501c0eadbc1ea17b3535da040a12ac7d281066e915eea";

/// A pipeline component that generates a Cairo PIE from a `BlockInfo { status: Mined }` and
/// emits `BlockInfo { status: SnosPieGenerated }` for the SNOS prover downstream.
///
/// This is the stage that was previously embedded inside `PollingBlockIngestor`. Extracting it
/// allows the block ingestor to remain proving-strategy-agnostic and makes it straightforward
/// to swap in a different preparation step (e.g. TEE attestation) without touching block control.
#[derive(Debug)]
pub struct SnosPieGenerator<DB> {
    rpc_url: Url,
    input_channel: Receiver<BlockInfo>,
    output_channel: Sender<BlockInfo>,
    finish_handle: FinishHandle,
    db: DB,
    workers_count: usize,
    os_hints_config: OsHintsConfiguration,
    chain_id: ChainId,
}

#[derive(Debug)]
pub struct SnosPieGeneratorBuilder<DB> {
    rpc_url: Url,
    input_channel: Option<Receiver<BlockInfo>>,
    output_channel: Option<Sender<BlockInfo>>,
    db: DB,
    workers_count: usize,
    os_hints_config: OsHintsConfiguration,
    chain_id: ChainId,
}

impl<DB> SnosPieGenerator<DB>
where
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    async fn worker(
        task_rx: Arc<Mutex<Receiver<BlockInfo>>>,
        task_tx: Sender<BlockInfo>,
        rpc_url: Url,
        finish_handle: FinishHandle,
        db: DB,
        os_hints_config: OsHintsConfiguration,
        chain_id: ChainId,
    ) {
        loop {
            let block_info = if let Some(b) = task_rx.lock().await.recv().await {
                b
            } else {
                break;
            };

            let block_number = block_info.number;
            let block_number_u32: u32 = block_number.try_into().unwrap();

            if finish_handle.is_shutdown_requested() {
                break;
            }

            // Resume: reuse an existing PIE from the DB if available.
            match db.get_pie(block_number_u32, Step::Snos).await {
                Ok(pie_bytes) => match CairoPie::from_bytes(&pie_bytes) {
                    Ok(_) => {
                        trace!(block_number, "SNOS PIE already in DB, skipping generation");
                        let out = BlockInfo {
                            number: block_number,
                            status: BlockStatus::SnosPieGenerated,
                            state_update: block_info.state_update,
                        };
                        if task_tx.send(out).await.is_err() {
                            error!(block_number, "Failed to forward block after PIE resume");
                        }
                        continue;
                    }
                    Err(err) => {
                        error!(block_number, error = %err, "Failed to parse existing PIE from DB");
                    }
                },
                Err(err) => {
                    trace!(block_number, error = %err, "SNOS PIE not found in DB, generating");
                }
            }

            let pie_input = generate_pie::types::PieGenerationInput {
                rpc_url: rpc_url.to_string(),
                blocks: vec![block_number],
                versioned_constants: None,
                chain_config: ChainConfig {
                    chain_id: chain_id.clone(),
                    strk_fee_token_address: contract_address!(KATANA_DEFAULT_TOKEN_ADDRESS),
                    is_l3: true,
                    eth_fee_token_address: contract_address!(KATANA_DEFAULT_TOKEN_ADDRESS),
                },
                layout: cairo_vm::types::layout_name::LayoutName::all_cairo,
                os_hints_config: os_hints_config.clone(),
                output_path: None,
            };

            let pie = generate_pie(pie_input).await.unwrap().output.cairo_pie;

            if finish_handle.is_shutdown_requested() {
                break;
            }

            let pie_bytes = compress_pie(pie).await.unwrap();
            db.add_pie(block_number_u32, pie_bytes, Step::Snos)
                .await
                .unwrap();

            info!(block_number, "SNOS PIE generated for block");

            let out = BlockInfo {
                number: block_number,
                status: BlockStatus::SnosPieGenerated,
                state_update: block_info.state_update,
            };

            if task_tx.send(out).await.is_err() {
                error!(block_number, "Failed to forward block after PIE generation");
            }
        }
    }

    async fn run(self) {
        let mut workers = Vec::new();
        let task_rx = Arc::new(Mutex::new(self.input_channel));

        for _ in 0..self.workers_count {
            workers.push(task::spawn(Self::worker(
                task_rx.clone(),
                self.output_channel.clone(),
                self.rpc_url.clone(),
                self.finish_handle.clone(),
                self.db.clone(),
                self.os_hints_config.clone(),
                self.chain_id.clone(),
            )));
        }

        futures_util::future::join_all(workers).await;
        debug!("Graceful shutdown finished");
        self.finish_handle.finish();
    }
}

impl<DB> SnosPieGeneratorBuilder<DB> {
    pub fn new(
        rpc_url: Url,
        db: DB,
        workers_count: usize,
        os_hints_config: OsHintsConfiguration,
        chain_id: ChainId,
    ) -> Self {
        Self {
            rpc_url,
            input_channel: None,
            output_channel: None,
            db,
            workers_count,
            os_hints_config,
            chain_id,
        }
    }
}

impl<DB> PipelineStageBuilder for SnosPieGeneratorBuilder<DB>
where
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    type Stage = SnosPieGenerator<DB>;

    fn build(self) -> Result<Self::Stage> {
        Ok(SnosPieGenerator {
            rpc_url: self.rpc_url,
            input_channel: self
                .input_channel
                .ok_or_else(|| anyhow::anyhow!("`input_channel` not set"))?,
            output_channel: self
                .output_channel
                .ok_or_else(|| anyhow::anyhow!("`output_channel` not set"))?,
            finish_handle: FinishHandle::new(),
            db: self.db,
            workers_count: self.workers_count,
            os_hints_config: self.os_hints_config,
            chain_id: self.chain_id,
        })
    }

    fn input_channel(mut self, input_channel: Receiver<BlockInfo>) -> Self {
        self.input_channel = Some(input_channel);
        self
    }

    fn output_channel(mut self, output_channel: Sender<BlockInfo>) -> Self {
        self.output_channel = Some(output_channel);
        self
    }
}

impl<DB> PipelineStage for SnosPieGenerator<DB>
where
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    type Input = BlockInfo;
    type Output = BlockInfo;
}

impl<DB> Daemon for SnosPieGenerator<DB>
where
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        tokio::spawn(self.run());
    }
}
