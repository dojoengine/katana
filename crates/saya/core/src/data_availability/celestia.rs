use anyhow::Result;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use celestia_rpc::{BlobClient, Client, TxConfig};
use celestia_types::{nmt::Namespace, AppVersion, Blob};
use starknet_types_core::felt::Felt;
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::{debug, info};
use url::Url;

use crate::{
    data_availability::{
        DataAvailabilityBackend, DataAvailabilityBackendBuilder, DataAvailabilityCursor,
        DataAvailabilityPacketContext, DataAvailabilityPayload, DataAvailabilityPointer,
    },
    service::{Daemon, FinishHandle, ShutdownHandle},
};

#[derive(Debug)]
pub struct CelestiaDataAvailabilityBackend<P> {
    rpc_url: Url,
    auth_token: String,
    namespace: Namespace,
    key_name: Option<String>,
    last_pointer: Option<DataAvailabilityPointer>,
    proof_channel: Receiver<P>,
    cursor_channel: Sender<DataAvailabilityCursor<P>>,
    finish_handle: FinishHandle,
}

#[derive(Debug)]
pub struct CelestiaDataAvailabilityBackendBuilder<P> {
    rpc_url: Url,
    auth_token: String,
    namespace: Namespace,
    key_name: Option<String>,
    last_pointer: Option<Option<DataAvailabilityPointer>>,
    proof_channel: Option<Receiver<P>>,
    cursor_channel: Option<Sender<DataAvailabilityCursor<P>>>,
}

impl<P> CelestiaDataAvailabilityBackend<P>
where
    P: DataAvailabilityPayload,
{
    async fn run(mut self) {
        loop {
            let new_proof = tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                new_proof = self.proof_channel.recv() => new_proof,
            };

            // This should be fine for now as provers wouldn't drop senders. This might change in
            // the future.
            let new_proof = new_proof.unwrap();
            debug!("Received new proof");

            // TODO: error handling
            let client = Client::new(self.rpc_url.as_ref(), Some(&self.auth_token), None, None)
                .await
                .unwrap();

            let packet = new_proof
                .clone()
                .into_packet(DataAvailabilityPacketContext {
                    prev: self.last_pointer,
                });

            // TODO: error handling
            let mut serialized_packet: Vec<u8> = Vec::new();
            ciborium::into_writer(&packet, &mut serialized_packet).unwrap();

            // TODO: error handling
            let blob = Blob::new(self.namespace, serialized_packet, None, AppVersion::V7).unwrap();
            let commitment = blob.clone().commitment;
            let commitment = commitment.hash();

            let tx_config = TxConfig {
                key_name: self.key_name.clone(),
                ..Default::default()
            };

            debug!(
                block_number = new_proof.block_number(),
                commitment = ?hex::encode(commitment),
                blob_bytes_size = blob.data.len(),
                "Submitting Celestia DA blob.",
            );

            // TODO: error handling
            let celestia_block = client.blob_submit(&[blob], tx_config).await.unwrap();

            self.last_pointer = Some(DataAvailabilityPointer {
                height: celestia_block,
                commitment: *commitment,
                namespace: Felt::from_bytes_be_slice(self.namespace.as_bytes()),
            });

            info!(
                block_number = new_proof.block_number(),
                celestia_block,
                namespace = ?STANDARD.encode(self.namespace.as_bytes()),
                commitment = ?hex::encode(commitment),
                "Blob posted on Celestia."
            );

            let new_cursor = DataAvailabilityCursor {
                block_number: new_proof.block_number(),
                pointer: Some(DataAvailabilityPointer {
                    height: celestia_block,
                    commitment: *commitment,
                    namespace: Felt::from_bytes_be_slice(self.namespace.as_bytes()),
                }),
                full_payload: new_proof,
            };

            // Since the channel is bounded, it's possible
            tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                _ = self.cursor_channel.send(new_cursor) => {},
            }
        }

        debug!("Graceful shutdown finished");
        self.finish_handle.finish();
    }
}

impl<P> CelestiaDataAvailabilityBackendBuilder<P> {
    pub fn new(
        rpc_url: Url,
        auth_token: String,
        namespace: String,
        key_name: Option<String>,
    ) -> Result<Self> {
        Ok(Self {
            rpc_url,
            auth_token,
            namespace: Namespace::new_v0(namespace.as_bytes())?,
            key_name,
            last_pointer: Some(None),
            proof_channel: None,
            cursor_channel: None,
        })
    }
}

impl<P> DataAvailabilityBackendBuilder for CelestiaDataAvailabilityBackendBuilder<P>
where
    P: DataAvailabilityPayload + 'static,
{
    type Backend = CelestiaDataAvailabilityBackend<P>;

    fn build(self) -> Result<Self::Backend> {
        Ok(CelestiaDataAvailabilityBackend {
            rpc_url: self.rpc_url,
            auth_token: self.auth_token,
            namespace: self.namespace,
            key_name: self.key_name,
            last_pointer: self
                .last_pointer
                .ok_or_else(|| anyhow::anyhow!("`last_pointer` not set"))?,
            proof_channel: self
                .proof_channel
                .ok_or_else(|| anyhow::anyhow!("`proof_channel` not set"))?,
            cursor_channel: self
                .cursor_channel
                .ok_or_else(|| anyhow::anyhow!("`cursor_channel` not set"))?,
            finish_handle: FinishHandle::new(),
        })
    }

    fn last_pointer(mut self, last_pointer: Option<DataAvailabilityPointer>) -> Self {
        self.last_pointer = Some(last_pointer);
        self
    }

    fn proof_channel(mut self, proof_channel: Receiver<P>) -> Self {
        self.proof_channel = Some(proof_channel);
        self
    }

    fn cursor_channel(mut self, cursor_channel: Sender<DataAvailabilityCursor<P>>) -> Self {
        self.cursor_channel = Some(cursor_channel);
        self
    }
}

impl<P> DataAvailabilityBackend for CelestiaDataAvailabilityBackend<P>
where
    P: DataAvailabilityPayload + 'static,
{
    type Payload = P;
}

impl<P> Daemon for CelestiaDataAvailabilityBackend<P>
where
    P: DataAvailabilityPayload + 'static,
{
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        tokio::spawn(self.run());
    }
}
