use super::{
    client::{AtlanticJobSize, AtlanticQueryResponse, AtlanticQueryStatus},
    AtlanticClient, AtlanticProof,
};
use crate::error::ProverError;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use saya_core::{
    prover::SnosProof,
    service::FinishHandle,
    storage::{PersistantStorage, Step},
};
use std::time::Duration;
use tracing::info;

const PROOF_STATUS_POLL_INTERVAL: Duration = Duration::from_secs(10);

/// Calculate the job size based on the number of steps in the pie.
/// Refer to the [Atlantic Prover](https://docs.herodotus.cloud/atlantic/sending-query) documentation for more details.
/// Larger sizes can be used for small pies, but this increases the cost.
/// The sizes affect the resources allocated to the job.
/// Available sizes are XS, S, M, and L. Size XS is purely virtual for Atlantic optimization and is interpreted as size S by SHARP.
/// While XS affects resource usage on the Atlantic backend, it has no impact on SHARP, and XS and S have the same cost in SHARP.
pub fn calculate_job_size(pie: CairoPie) -> AtlanticJobSize {
    match pie.execution_resources.n_steps {
        0..=6_499_999 => AtlanticJobSize::XS,
        6_500_000..=12_999_999 => AtlanticJobSize::S,
        13_000_000..=29_999_999 => AtlanticJobSize::M,
        _ => AtlanticJobSize::L,
    }
}

pub async fn wait_for_query(
    client: AtlanticClient,
    atlantic_query_id: String,
    finish_handle: FinishHandle,
) -> Result<AtlanticQueryResponse, ProverError> {
    let response = loop {
        tokio::time::sleep(PROOF_STATUS_POLL_INTERVAL).await;

        if finish_handle.is_shutdown_requested() {
            return Err(ProverError::Shutdown);
        }

        if let Ok(query) = client.clone().get_atlantic_query(&atlantic_query_id).await {
            match query.atlantic_query.status {
                AtlanticQueryStatus::Done => break query,
                AtlanticQueryStatus::Failed => {
                    return Err(ProverError::BlockFail(format!(
                        "Proof generation failed for query: {}",
                        atlantic_query_id
                    )));
                }
                _ => continue,
            }
        }
    };
    Ok(response)
}

pub async fn parse_and_store_proof<P, DB>(
    raw_proof: String,
    db: DB,
    block_number: u32,
    step: Step,
) -> Result<SnosProof<P>, ProverError>
where
    P: AtlanticProof + Send + Sync + 'static,
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    let proof_in_bytes = raw_proof.as_bytes().to_vec();

    db.add_proof(block_number, proof_in_bytes.clone(), step)
        .await
        .unwrap();

    //Sanity check to ensure that the proof can be parsed and is valid
    let parsed_proof: P = P::parse(raw_proof).map_err(|e| {
        ProverError::ProofParse(format!(
            "Failed to parse proof for block number {}: {}",
            block_number, e
        ))
    })?;

    info!(block_number, "SNOS proof successfully retrieved from Atlantic.");

    Ok(SnosProof {
        block_number: block_number as u64,
        proof: parsed_proof,
    })
}
