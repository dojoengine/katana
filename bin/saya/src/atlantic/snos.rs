use std::{io::Write, sync::Arc, time::Duration};

use anyhow::Result;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use starknet::core::types::Felt;
use tokio::{
    sync::{
        mpsc::{Receiver, Sender},
        Mutex,
    },
    task,
};
use tracing::{debug, error, info, trace};
use zip::{write::FileOptions, ZipWriter};

use crate::{
    atlantic::{
        client::{AtlanticClient, Layout},
        shared::{calculate_job_size, parse_and_store_proof, wait_for_query},
        AtlanticProof,
    },
    error::ProverError,
    utils::{compute_program_hash_from_pie, extract_pie_output, stark_proof_mock},
};
use saya_core::{
    block_ingestor::BlockInfo,
    prover::{PipelineStage, PipelineStageBuilder, SnosProof},
    service::{Daemon, FinishHandle, ShutdownHandle},
    storage::{PersistantStorage, Step},
};
/// Prover implementation as a client to the hosted [Atlantic Prover](https://atlanticprover.com/)
/// service.
#[derive(Debug)]
pub struct AtlanticSnosProver<P, DB> {
    client: AtlanticClient,
    input_channel: Receiver<BlockInfo>,
    output_channel: Sender<SnosProof<P>>,
    finish_handle: FinishHandle,
    /// Whether to extract the output and compute the program hash from the PIE or use the one from the SHARP bootloader returned by the prover service.
    mock_snos_from_pie: bool,
    db: DB,
    worker_count: usize,
}

#[derive(Debug)]
pub struct AtlanticSnosProverBuilder<P, DB> {
    api_key: String,
    input_channel: Option<Receiver<BlockInfo>>,
    output_channel: Option<Sender<SnosProof<P>>>,
    mock_snos_from_pie: bool,
    db: DB,
    worker_count: usize,
}

impl<P, DB> AtlanticSnosProver<P, DB>
where
    P: AtlanticProof + Send + Sync + 'static,
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    async fn worker(
        task_rx: Arc<Mutex<Receiver<BlockInfo>>>,
        task_tx: Sender<SnosProof<P>>,
        client: AtlanticClient,
        finish_handle: FinishHandle,
        mock_snos_from_pie: bool,
        db: DB,
    ) -> Result<(), ProverError>
    where
        P: AtlanticProof,
        DB: PersistantStorage,
    {
        loop {
            let new_block = if let Some(new_block) = task_rx.lock().await.recv().await {
                new_block
            } else {
                break;
            };
            let block_number_u32 = new_block.number.try_into().map_err(|_| {
                ProverError::Prover("Block number too large to fit in u32".to_string())
            })?;

            match db
                .get_proof(block_number_u32, saya_core::storage::Step::Snos)
                .await
            {
                Ok(proof) => {
                    info!(
                        block_number = new_block.number,
                        "Proof already generated for block"
                    );
                    let raw_proof = String::from_utf8(proof).unwrap();
                    let parsed_proof: P = P::parse(raw_proof).unwrap();
                    let new_proof = SnosProof {
                        block_number: new_block.number,
                        proof: parsed_proof,
                    };
                    let _ = task_tx.send(new_proof).await;
                    continue;
                }
                Err(_) => {
                    trace!(
                        block_number = block_number_u32,
                        "Proof not found in db for block",
                    );
                }
            }

            if mock_snos_from_pie {
                Self::mock_proof(new_block, task_tx.clone(), db.clone())
                    .await
                    .unwrap();
                continue;
            }

            match db
                .get_query_id(block_number_u32, saya_core::storage::Query::SnosProof)
                .await
            {
                Ok(atlantic_query_id) => {
                    info!(
                        block_number = new_block.number,
                        atlantic_query_id = %atlantic_query_id,
                        "Atlantic proof generation already submitted for block",
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

                    let raw_proof = query_response.get_proof(&client).await?;

                    let new_proof =
                        parse_and_store_proof(raw_proof, db.clone(), block_number_u32, Step::Snos)
                            .await?;

                    tokio::select! {
                        _ = finish_handle.shutdown_requested() => break,
                        _ = task_tx.send(new_proof) => {},
                    }
                    continue;
                }
                Err(_) => {
                    // Not found in db, we continue.
                }
            }
            // TODO: error handling

            let compressed_pie: Vec<u8> = db
                .get_pie(block_number_u32, saya_core::storage::Step::Snos)
                .await
                .unwrap();

            debug!(
                "Compressed PIE size for block #{}: {} bytes",
                new_block.number,
                compressed_pie.len()
            );
            let atlantic_job_size =
                calculate_job_size(CairoPie::from_bytes(&compressed_pie).unwrap());
            let atlantic_query_id = crate::utils::retry_with_backoff(
                || {
                    client.submit_proof_generation(
                        compressed_pie.clone(),
                        Layout::dynamic,
                        format!("snos_{}", new_block.number),
                        atlantic_job_size,
                    )
                },
                "submit_proof_generation",
                3,
                Duration::from_secs(5),
            )
            .await?;

            db.add_query_id(
                new_block.number.try_into().unwrap(),
                atlantic_query_id.clone(),
                saya_core::storage::Query::SnosProof,
            )
            .await
            .unwrap();

            info!(
                block_number = new_block.number,
                atlantic_query_id = %atlantic_query_id,
                "Atlantic proof generation submitted for block"
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

            debug!(
                "Atlantic PIE proof generation finished for query: {}",
                atlantic_query_id
            );
            let raw_proof = query_response.get_proof(&client).await?;

            let new_proof =
                parse_and_store_proof(raw_proof, db.clone(), block_number_u32, Step::Snos).await?;

            tokio::select! {
                _ = finish_handle.shutdown_requested() => break,
                _ = task_tx.send(new_proof) => {},
            }
        }
        Ok(())
    }

    async fn run(self) {
        let mut workers = Vec::new();
        let task_rx = Arc::new(Mutex::new(self.input_channel));
        for _ in 0..self.worker_count {
            let worker_task_tx = self.output_channel.clone();
            workers.push(task::spawn(Self::worker(
                task_rx.clone(),
                worker_task_tx,
                self.client.clone(),
                self.finish_handle.clone(),
                self.mock_snos_from_pie,
                self.db.clone(),
            )));
        }
        futures_util::future::join_all(workers).await;
        debug!("Graceful shutdown finished");
        self.finish_handle.finish();
    }

    async fn mock_proof(new_block: BlockInfo, task_tx: Sender<SnosProof<P>>, db: DB) -> Result<()> {
        let pie = db
            .get_pie(
                new_block.number.try_into().unwrap(),
                saya_core::storage::Step::Snos,
            )
            .await
            .unwrap();
        let cairo_pie = CairoPie::from_bytes(&pie).unwrap();
        let output = bootloader_snos_output(&cairo_pie);
        let mock_proof = stark_proof_mock(&output);

        info!(
            block_number = new_block.number,
            "Mock proof generated from PIE",
        );

        let new_proof = SnosProof {
            block_number: new_block.number,
            proof: P::parse(serde_json::to_string(&mock_proof).unwrap()).unwrap(),
        };

        let _ = task_tx.send(new_proof).await;
        Ok(())
    }
}

impl<P, DB> AtlanticSnosProverBuilder<P, DB> {
    pub fn new(api_key: String, mock_snos_from_pie: bool, db: DB, worker_count: usize) -> Self {
        Self {
            api_key,
            input_channel: None,
            output_channel: None,
            mock_snos_from_pie,
            db,
            worker_count,
        }
    }
}

impl<P, DB> PipelineStageBuilder for AtlanticSnosProverBuilder<P, DB>
where
    P: AtlanticProof + Send + Sync + 'static,
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    type Stage = AtlanticSnosProver<P, DB>;

    fn build(self) -> Result<Self::Stage> {
        Ok(AtlanticSnosProver {
            client: AtlanticClient::new(self.api_key),
            input_channel: self
                .input_channel
                .ok_or_else(|| anyhow::anyhow!("`input_channel` not set"))?,
            output_channel: self
                .output_channel
                .ok_or_else(|| anyhow::anyhow!("`output_channel` not set"))?,
            finish_handle: FinishHandle::new(),
            mock_snos_from_pie: self.mock_snos_from_pie,
            db: self.db,
            worker_count: self.worker_count,
        })
    }

    fn input_channel(mut self, input_channel: Receiver<BlockInfo>) -> Self {
        self.input_channel = Some(input_channel);
        self
    }

    fn output_channel(mut self, output_channel: Sender<SnosProof<P>>) -> Self {
        self.output_channel = Some(output_channel);
        self
    }
}

impl<P, DB> PipelineStage for AtlanticSnosProver<P, DB>
where
    P: AtlanticProof + Send + Sync + 'static,
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    type Input = BlockInfo;
    type Output = SnosProof<P>;
}

impl<P, DB> Daemon for AtlanticSnosProver<P, DB>
where
    P: AtlanticProof + Send + Sync + 'static,
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        tokio::spawn(self.run());
    }
}

/// Converts a `CairoPie` instance into a Zip archive accepted by the Atlantic prover service.
///
/// Unfortunately `cairo-vm` does not offer a generic API for converting PIE to Zip bytes that
/// doesn't involve using the file system. This is mostly copied from `CairoPie::write_zip_file`.
pub async fn compress_pie(pie: CairoPie) -> std::result::Result<Vec<u8>, std::io::Error> {
    task::spawn_blocking(move || {
        let mut bytes = std::io::Cursor::new(Vec::<u8>::new());
        let mut zip_writer = ZipWriter::new(&mut bytes);
        let options =
            FileOptions::<'_, ()>::default().compression_method(zip::CompressionMethod::Deflated);

        zip_writer.start_file("version.json", options)?;
        serde_json::to_writer(&mut zip_writer, &pie.version)?;
        zip_writer.start_file("metadata.json", options)?;
        serde_json::to_writer(&mut zip_writer, &pie.metadata)?;
        zip_writer.start_file("memory.bin", options)?;
        zip_writer.write_all(&pie.memory.to_bytes(None))?;
        zip_writer.start_file("additional_data.json", options)?;
        serde_json::to_writer(&mut zip_writer, &pie.additional_data)?;
        zip_writer.start_file("execution_resources.json", options)?;
        serde_json::to_writer(&mut zip_writer, &pie.execution_resources)?;
        zip_writer.finish()?;

        Ok(bytes.into_inner())
    })
    .await?
}

/// Mocks a bootloaded execution of SNOS.
fn bootloader_snos_output(pie: &CairoPie) -> Vec<Felt> {
    let snos_program_hash =
        compute_program_hash_from_pie(pie).expect("Failed to compute program hash from PIE");
    debug!(snos_program_hash = %snos_program_hash, "SNOS program hash from PIE");

    let snos_output = extract_pie_output(pie);

    let mut bootloader_output = vec![
        // Bootloader constants (number of task executed by bootloader, in case of herodotus its always 1)
        Felt::ONE,
        // bootloader output len (not checked by piltover, set to 0)
        Felt::ZERO,
        snos_program_hash,
    ];

    bootloader_output.extend(snos_output);
    bootloader_output
}
