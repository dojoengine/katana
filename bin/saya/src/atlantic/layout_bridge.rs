use std::{borrow::Cow, sync::Arc, time::Duration};

use crate::{
    atlantic::{
        client::{AtlanticClient, Layout},
        shared::{calculate_job_size, parse_and_store_proof, wait_for_query},
        snos::compress_pie,
    },
    error::ProverError,
};
use anyhow::Result;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use saya_core::{
    block_ingestor::BlockInfo,
    prover::{PipelineStage, PipelineStageBuilder, SnosProof},
    service::{Daemon, FinishHandle, ShutdownHandle},
    storage::{PersistantStorage, Step},
};
use tokio::sync::{
    mpsc::{Receiver, Sender},
    Mutex,
};
use tracing::{debug, error, info, trace, warn};
/// Prover implementation as a client to the hosted [Atlantic Prover](https://atlanticprover.com/)
/// service.
#[derive(Debug)]
pub struct AtlanticLayoutBridgeProver<DB> {
    client: AtlanticClient,
    layout_bridge: Cow<'static, [u8]>,
    input_channel: Receiver<SnosProof<String>>,
    output_channel: Sender<BlockInfo>,
    finish_handle: FinishHandle,
    db: DB,
    workers_count: usize,
}

#[derive(Debug)]
pub struct AtlanticLayoutBridgeProverBuilder<DB> {
    api_key: String,
    layout_bridge: Cow<'static, [u8]>,
    input_channel: Option<Receiver<SnosProof<String>>>,
    output_channel: Option<Sender<BlockInfo>>,
    db: DB,
    workers_count: usize,
}

impl<DB> AtlanticLayoutBridgeProver<DB>
where
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    async fn worker(
        task_rx: Arc<Mutex<Receiver<SnosProof<String>>>>,
        task_tx: Sender<BlockInfo>,
        client: AtlanticClient,
        layout_bridge: Cow<'static, [u8]>,
        finish_handle: FinishHandle,
        db: DB,
    ) -> Result<(), ProverError>
    where
        DB: PersistantStorage + Send + Sync + 'static,
    {
        loop {
            let new_snos_proof = if let Some(new_block) = task_rx.lock().await.recv().await {
                new_block
            } else {
                break;
            };

            let block_number_u32 = new_snos_proof.block_number.try_into().unwrap();
            let state_update = db
                .get_state_update(block_number_u32)
                .await
                .map_err(|e| ProverError::MetadataFetch(e.to_string()))?;
            match db
                .get_proof(block_number_u32, saya_core::storage::Step::Bridge)
                .await
            {
                Ok(proof) => {
                    let verifier_proof = String::from_utf8(proof).unwrap();

                    // Sanity check if the proof is valid.
                    let proof = swiftness::parse(verifier_proof);
                    if proof.is_ok() {
                        trace!(
                            block_number = new_snos_proof.block_number,
                            "Proof already generated for block"
                        );
                        let block_info = BlockInfo {
                            number: new_snos_proof.block_number,
                            status: saya_core::storage::BlockStatus::SnosProofGenerated,
                            state_update: Some(state_update.clone()),
                        };

                        task_tx.send(block_info).await.unwrap();
                        continue;
                    } else {
                        // TODO: ensure the following match on the `get_query_id` isn't conflicting with this situation.
                        warn!(
                            block_number = new_snos_proof.block_number,
                            "Invalid proof found in db, not using proof from db.",
                        );
                    }
                }
                Err(_) => {
                    trace!(
                        block_number = new_snos_proof.block_number,
                        "Proof not found in db for block",
                    );
                }
            }

            match db
                .get_query_id(block_number_u32, saya_core::storage::Query::BridgeProof)
                .await
            {
                Ok(atlantic_query_id) => {
                    info!(
                        block_number = new_snos_proof.block_number,
                        "Proof generation already submitted for block"
                    );
                    let query_response = match wait_for_query(
                        client.clone(),
                        atlantic_query_id.clone(),
                        finish_handle.clone(),
                    )
                    .await
                    {
                        Err(ProverError::Shutdown) => {
                            break;
                        }
                        Err(ProverError::BlockFail(e)) => {
                            error!(error = %e, atlantic_query_id = %atlantic_query_id, "Proof generation failed");
                            db.add_failed_block(block_number_u32, e).await.unwrap();
                            continue;
                        }
                        Err(e) => {
                            error!(
                                "Unreachable error: {:?} while processing query {}",
                                e, atlantic_query_id
                            );
                            unreachable!("Unexpected ProverError: {:?}", e);
                        }
                        Ok(response) => response,
                    };

                    debug!(
                        atlantic_query_id = ?atlantic_query_id,
                        "Atlantic layout bridge proof generation finished"
                    );

                    let raw_proof = query_response.get_proof(&client).await?;

                    let _: SnosProof<String> = parse_and_store_proof(
                        raw_proof,
                        db.clone(),
                        block_number_u32,
                        Step::Bridge,
                    )
                    .await?;

                    let output = BlockInfo {
                        number: new_snos_proof.block_number,
                        status: saya_core::storage::BlockStatus::SnosProofGenerated,
                        state_update: Some(state_update.clone()),
                    };

                    task_tx.send(output).await.unwrap();
                    continue;
                }
                Err(_) => {
                    trace!(
                        block_number = new_snos_proof.block_number,
                        "Proof generation not submitted for block"
                    );
                }
            }

            let compressed_pie = match db.get_pie(block_number_u32, Step::Bridge).await {
                Ok(pie) => pie,
                Err(_) => {
                    // Hacky way to wrap proof due to the lack of serialization support for the parsed type4
                    // TODO: patch `swiftness` and fix this
                    let input = format!("{{\n\t\"proof\": {}\n}}", new_snos_proof.proof);
                    let label = format!("layout-trace-{}", new_snos_proof.block_number);

                    let atlantic_query_id = match db
                        .get_query_id(block_number_u32, saya_core::storage::Query::BridgeTrace)
                        .await
                    {
                        Ok(query_id) => query_id,
                        Err(_) => {
                            let atlantic_query_id = crate::utils::retry_with_backoff(
                                || {
                                    client.submit_trace_generation(
                                        &label,
                                        layout_bridge.clone().to_vec(),
                                        input.clone().into_bytes(),
                                    )
                                },
                                "trace_gen",
                                3,
                                Duration::from_secs(5),
                            )
                            .await
                            .unwrap();

                            db.add_query_id(
                                block_number_u32,
                                atlantic_query_id.clone(),
                                saya_core::storage::Query::BridgeTrace,
                            )
                            .await
                            .unwrap();

                            atlantic_query_id
                        }
                    };

                    info!(
                        block_number = new_snos_proof.block_number,
                        atlantic_query_id = ?atlantic_query_id,
                        "Atlantic trace generation submitted",
                    );

                    let query_response = match wait_for_query(
                        client.clone(),
                        atlantic_query_id.clone(),
                        finish_handle.clone(),
                    )
                    .await
                    {
                        Err(ProverError::Shutdown) => {
                            break;
                        }
                        Err(ProverError::BlockFail(e)) => {
                            error!("{}", e);
                            db.add_failed_block(block_number_u32, e).await.unwrap();
                            continue;
                        }
                        Err(e) => {
                            error!(
                                "Unreachable error: {:?} while processing query {}",
                                e, atlantic_query_id
                            );
                            unreachable!("Unexpected ProverError: {:?}", e);
                        }
                        Ok(response) => response,
                    };

                    let pie_bytes = query_response.get_pie(&client).await?;
                    let layout_bridge_pie = CairoPie::from_bytes(&pie_bytes).unwrap();

                    let compressed_pie = compress_pie(layout_bridge_pie).await.unwrap();

                    db.add_pie(block_number_u32, compressed_pie.clone(), Step::Bridge)
                        .await
                        .unwrap();

                    compressed_pie
                }
            };
            let atlantic_job_size =
                calculate_job_size(CairoPie::from_bytes(&compressed_pie).unwrap());
            let atlantic_query_id = crate::utils::retry_with_backoff(
                || {
                    client.submit_proof_generation(
                        compressed_pie.clone(),
                        Layout::recursive_with_poseidon,
                        format!("layout-{}", new_snos_proof.block_number),
                        atlantic_job_size,
                    )
                },
                "submit_proof_generation",
                3,
                Duration::from_secs(5),
            )
            .await
            .unwrap();

            db.add_query_id(
                new_snos_proof.block_number.try_into().unwrap(),
                atlantic_query_id.clone(),
                saya_core::storage::Query::BridgeProof,
            )
            .await
            .unwrap();

            info!(
                block_number = new_snos_proof.block_number,
                atlantic_query_id = ?atlantic_query_id,
                "Atlantic layout bridge proof generation submitted",
            );

            // Wait for bridge layout proof to be done
            let query_response = match wait_for_query(
                client.clone(),
                atlantic_query_id.clone(),
                finish_handle.clone(),
            )
            .await
            {
                Err(ProverError::Shutdown) => break,
                Err(ProverError::BlockFail(e)) => {
                    error!(error = %e, atlantic_query_id = %atlantic_query_id, "Proof generation failed");
                    db.add_failed_block(block_number_u32, e).await.unwrap();
                    continue;
                }
                Err(e) => {
                    error!(
                        "Unreachable error: {:?} while processing query {}",
                        e, atlantic_query_id
                    );
                    unreachable!("Unexpected ProverError: {:?}", e);
                }
                Ok(response) => response,
            };
            let raw_proof = query_response.get_proof(&client).await?;

            let _: SnosProof<String> =
                parse_and_store_proof(raw_proof, db.clone(), block_number_u32, Step::Bridge)
                    .await
                    .unwrap();

            debug!(
                block_number = new_snos_proof.block_number,
                atlantic_query_id = ?atlantic_query_id,
                "Atlantic layout bridge proof generation finished",
            );

            let output = BlockInfo {
                number: new_snos_proof.block_number,
                status: saya_core::storage::BlockStatus::SnosProofGenerated,
                state_update: Some(state_update.clone()),
            };

            tokio::select! {
                _ = finish_handle.shutdown_requested() => break,
                _ = task_tx.send(output) => {},
            }
        }
        Ok(())
    }

    async fn run(self) {
        let mut workers = Vec::new();
        let task_rx = Arc::new(Mutex::new(self.input_channel));
        for _ in 0..self.workers_count {
            let worker_task_rx = task_rx.clone();
            let task_tx = self.output_channel.clone();
            let client = self.client.clone();
            let layout_bridge = self.layout_bridge.clone();
            let finish_handle = self.finish_handle.clone();
            workers.push(tokio::spawn(Self::worker(
                worker_task_rx,
                task_tx,
                client,
                layout_bridge,
                finish_handle,
                self.db.clone(),
            )));
        }
        futures_util::future::join_all(workers).await;

        debug!("Graceful shutdown finished");
        self.finish_handle.finish();
    }
}

impl<DB> AtlanticLayoutBridgeProverBuilder<DB> {
    pub fn new<P>(api_key: String, layout_bridge: P, db: DB, workers_count: usize) -> Self
    where
        P: Into<Cow<'static, [u8]>>,
        DB: PersistantStorage + Send + Sync + Clone + 'static,
    {
        Self {
            api_key,
            layout_bridge: layout_bridge.into(),
            input_channel: None,
            output_channel: None,
            db,
            workers_count,
        }
    }
}

impl<DB> PipelineStageBuilder for AtlanticLayoutBridgeProverBuilder<DB>
where
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    type Stage = AtlanticLayoutBridgeProver<DB>;

    fn build(self) -> Result<Self::Stage> {
        Ok(AtlanticLayoutBridgeProver {
            client: AtlanticClient::new(self.api_key),
            layout_bridge: self.layout_bridge,
            input_channel: self
                .input_channel
                .ok_or_else(|| anyhow::anyhow!("`input_channel` not set"))?,
            output_channel: self
                .output_channel
                .ok_or_else(|| anyhow::anyhow!("`output_channel` not set"))?,
            finish_handle: FinishHandle::new(),
            db: self.db,
            workers_count: self.workers_count,
        })
    }

    fn input_channel(mut self, input_channel: Receiver<SnosProof<String>>) -> Self {
        self.input_channel = Some(input_channel);
        self
    }

    fn output_channel(mut self, output_channel: Sender<BlockInfo>) -> Self {
        self.output_channel = Some(output_channel);
        self
    }
}

impl<DB> PipelineStage for AtlanticLayoutBridgeProver<DB>
where
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    type Input = SnosProof<String>;
    type Output = BlockInfo;
}

impl<DB> Daemon for AtlanticLayoutBridgeProver<DB>
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
