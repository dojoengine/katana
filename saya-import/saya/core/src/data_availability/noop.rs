use anyhow::Result;
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::debug;

use crate::{
    data_availability::{
        DataAvailabilityBackend, DataAvailabilityBackendBuilder, DataAvailabilityCursor,
        DataAvailabilityPayload, DataAvailabilityPointer,
    },
    service::{Daemon, FinishHandle, ShutdownHandle},
};

/// A placeholder to fill the gap where a data availability backend is expected but no data needs to
/// be published.
///
/// Upon receiving a payload, this backend immediately delivers it to the output channel.
#[derive(Debug)]
pub struct NoopDataAvailabilityBackend<P> {
    proof_channel: Receiver<P>,
    cursor_channel: Sender<DataAvailabilityCursor<P>>,
    finish_handle: FinishHandle,
}

#[derive(Debug, Default)]
pub struct NoopDataAvailabilityBackendBuilder<P> {
    proof_channel: Option<Receiver<P>>,
    cursor_channel: Option<Sender<DataAvailabilityCursor<P>>>,
}

impl<P> NoopDataAvailabilityBackend<P>
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

            let new_cursor = DataAvailabilityCursor {
                block_number: new_proof.block_number(),
                pointer: None,
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

impl<P> NoopDataAvailabilityBackendBuilder<P> {
    pub fn new() -> Self {
        Self {
            proof_channel: None,
            cursor_channel: None,
        }
    }
}

impl<P> DataAvailabilityBackendBuilder for NoopDataAvailabilityBackendBuilder<P>
where
    P: DataAvailabilityPayload + 'static,
{
    type Backend = NoopDataAvailabilityBackend<P>;

    fn build(self) -> Result<Self::Backend> {
        Ok(NoopDataAvailabilityBackend {
            proof_channel: self
                .proof_channel
                .ok_or_else(|| anyhow::anyhow!("`proof_channel` not set"))?,
            cursor_channel: self
                .cursor_channel
                .ok_or_else(|| anyhow::anyhow!("`cursor_channel` not set"))?,
            finish_handle: FinishHandle::new(),
        })
    }

    fn last_pointer(self, _last_pointer: Option<DataAvailabilityPointer>) -> Self {
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

impl<P> DataAvailabilityBackend for NoopDataAvailabilityBackend<P>
where
    P: DataAvailabilityPayload + 'static,
{
    type Payload = P;
}

impl<P> Daemon for NoopDataAvailabilityBackend<P>
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
