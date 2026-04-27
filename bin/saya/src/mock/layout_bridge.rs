use anyhow::Result;
use integrity::Felt;
use swiftness::TransformTo;
use swiftness_stark::types::StarkProof;
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::{debug, info};

use crate::utils::{calculate_output, stark_proof_mock};
use saya_core::{
    block_ingestor::BlockInfo,
    prover::{PipelineStage, PipelineStageBuilder, SnosProof},
    service::{Daemon, FinishHandle, ShutdownHandle},
    storage::PersistantStorage,
};
/// Prover implementation as a client to the hosted [Mock Prover](https://docs.herodotus.cloud/atlantic-api/introduction)
/// service.
#[derive(Debug)]
pub struct MockLayoutBridgeProver<DB> {
    input_channel: Receiver<SnosProof<String>>,
    output_channel: Sender<BlockInfo>,
    layout_bridge_program_hash: Felt,
    finish_handle: FinishHandle,
    db: DB,
}

#[derive(Debug, Default)]
pub struct MockLayoutBridgeProverBuilder<DB> {
    input_channel: Option<Receiver<SnosProof<String>>>,
    output_channel: Option<Sender<BlockInfo>>,
    layout_bridge_program_hash: Felt,
    db: DB,
}

impl<DB> MockLayoutBridgeProver<DB>
where
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    async fn run(mut self) {
        loop {
            let new_snos_proof = tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                new_snos_proof = self.input_channel.recv() => new_snos_proof,
            };
            // This should be fine for now as block ingestors wouldn't drop senders. This might
            // change in the future.
            let new_snos_proof = new_snos_proof.unwrap();
            let state_update = self
                .db
                .get_state_update(new_snos_proof.block_number.try_into().unwrap())
                .await
                .unwrap();

            debug!(
                "Receive raw SNOS proof for block #{}",
                new_snos_proof.block_number
            );

            // TODO: error handling
            let parsed_snos_proof: StarkProof = match swiftness::parse(&new_snos_proof.proof) {
                Ok(proof) => proof.transform_to(),
                Err(_) => {
                    // If the proof is sent by a mocked SNOS, it's already in the correct format.
                    serde_json::from_str::<StarkProof>(&new_snos_proof.proof).unwrap()
                }
            };

            let snos_output = calculate_output(&parsed_snos_proof);

            let mut bootloader_output = vec![
                // Bootloader constants (number of task executed by bootloader, in case of herodotus its always 1)
                Felt::ONE,
                // Bootloader output len (not checked by piltover, set to 0)
                Felt::ZERO,
                // Verifier Program Hash (not checked by piltover, set to 0)
                self.layout_bridge_program_hash,
                // Bootloader program hash
                Felt::from_hex_unchecked(
                    "0x5ab580b04e3532b6b18f81cfa654a05e29dd8e2352d88df1e765a84072db07",
                ),
                // Verifier output len (not checked by piltover, set to 0)
                Felt::ZERO,
            ];
            bootloader_output.extend_from_slice(&snos_output);

            let mock_proof = stark_proof_mock(&bootloader_output);

            let string_proof = serde_json::to_string(&mock_proof).unwrap();
            let bytes_proof = string_proof.as_bytes();

            self.db
                .add_proof(
                    new_snos_proof.block_number.try_into().unwrap(),
                    bytes_proof.to_vec(),
                    saya_core::storage::Step::Bridge,
                )
                .await
                .unwrap();

            info!(
                "Mock proof generated for block #{}",
                new_snos_proof.block_number
            );
            let output = BlockInfo {
                number: new_snos_proof.block_number,
                status: saya_core::storage::BlockStatus::BridgeProofGenerated,
                state_update: Some(state_update),
            };
            tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                _ = self.output_channel.send(output) => {},
            }
        }

        debug!("Graceful shutdown finished");
        self.finish_handle.finish();
    }
}

impl<DB> MockLayoutBridgeProverBuilder<DB> {
    pub fn new(layout_bridge_program_hash: Felt, db: DB) -> Self {
        Self {
            input_channel: None,
            output_channel: None,
            layout_bridge_program_hash,
            db,
        }
    }
}

impl<DB> PipelineStageBuilder for MockLayoutBridgeProverBuilder<DB>
where
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    type Stage = MockLayoutBridgeProver<DB>;

    fn build(self) -> Result<Self::Stage> {
        Ok(MockLayoutBridgeProver {
            input_channel: self
                .input_channel
                .ok_or_else(|| anyhow::anyhow!("`input_channel` not set"))?,
            output_channel: self
                .output_channel
                .ok_or_else(|| anyhow::anyhow!("`output_channel` not set"))?,
            finish_handle: FinishHandle::new(),
            layout_bridge_program_hash: self.layout_bridge_program_hash,
            db: self.db,
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

impl<DB> PipelineStage for MockLayoutBridgeProver<DB>
where
    DB: PersistantStorage + Send + Sync + Clone + 'static,
{
    type Input = SnosProof<String>;
    type Output = BlockInfo;
}

impl<DB> Daemon for MockLayoutBridgeProver<DB>
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
