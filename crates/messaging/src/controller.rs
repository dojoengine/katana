//! RPC-facing checkpoint controller for the messaging service.
//!
//! Encapsulates DB-side checkpoint operations and signalling the running drain
//! task to live-rewind the in-memory cursor without restarting the node.

use anyhow::Context;
use katana_provider::api::messaging::{
    MessagingCheckpoint, MessagingCheckpointProvider, MessagingL1ToL2IndexWriter,
};
use katana_provider::{MutableProvider, ProviderFactory, ProviderRW, ProviderResult};
use tokio::sync::mpsc;
use tracing::warn;

use crate::service::CHECKPOINT_ID;
use crate::LOG_TARGET;

/// Signal sent from the controller to the drain task: rewind the in-memory
/// cursor to `(from_block, from_tx_index)`.
#[derive(Debug, Clone, Copy)]
pub struct RewindSignal {
    pub from_block: u64,
    pub from_tx_index: u64,
}

/// Operator-facing handle to the messaging checkpoint.
///
/// Reads/writes the persisted DB checkpoint and signals the running drain
/// task to rewind its in-memory cursor.
#[derive(Debug, Clone)]
pub struct MessagingController<P> {
    provider: P,
    default_from_block: u64,
    rewind_tx: mpsc::Sender<RewindSignal>,
}

impl<P> MessagingController<P> {
    pub(crate) fn new(
        provider: P,
        default_from_block: u64,
        rewind_tx: mpsc::Sender<RewindSignal>,
    ) -> Self {
        Self { provider, default_from_block, rewind_tx }
    }
}

impl<P> MessagingController<P>
where
    P: ProviderFactory + Clone + Send + Sync + 'static,
    <P as ProviderFactory>::ProviderMut:
        ProviderRW + MessagingCheckpointProvider + MessagingL1ToL2IndexWriter + MutableProvider,
{
    /// Read the last *committed* checkpoint — the same value `resume_cursor`
    /// reads on boot.
    pub fn get_checkpoint(&self) -> ProviderResult<Option<MessagingCheckpoint>> {
        let db_tx = self.provider.provider_mut();
        let cp = db_tx.messaging_checkpoint(CHECKPOINT_ID)?;
        MutableProvider::commit(db_tx)?;
        Ok(cp)
    }

    /// Persist `(block, tx_index)` as the last processed checkpoint and signal
    /// the drain task to rewind the in-memory cursor to `(block, tx_index + 1)`.
    pub async fn set_checkpoint(&self, block: u64, tx_index: u64) -> anyhow::Result<()> {
        let db_tx = self.provider.provider_mut();
        db_tx
            .set_messaging_checkpoint(CHECKPOINT_ID, &MessagingCheckpoint { block, tx_index })
            .context("set messaging checkpoint")?;
        MutableProvider::commit(db_tx).context("commit checkpoint write")?;

        // The DB write is the source of truth — a failed channel send (server
        // not running, or already stopped) is logged but does not fail the call.
        // The next `start()` will resume from the new value.
        let signal = RewindSignal { from_block: block, from_tx_index: tx_index + 1 };
        if let Err(e) = self.rewind_tx.send(signal).await {
            warn!(
                target: LOG_TARGET,
                error = %e,
                "Failed to send rewind signal; DB checkpoint persisted, will be picked up on next start.",
            );
        }
        Ok(())
    }

    /// Delete the persisted checkpoint and signal the drain task to rewind to
    /// the configured `default_from_block` (the value used by `resume_cursor`
    /// when no checkpoint exists).
    pub async fn reset_checkpoint(&self) -> anyhow::Result<()> {
        let db_tx = self.provider.provider_mut();
        db_tx.delete_messaging_checkpoint(CHECKPOINT_ID).context("delete messaging checkpoint")?;
        MutableProvider::commit(db_tx).context("commit checkpoint delete")?;

        let signal = RewindSignal { from_block: self.default_from_block, from_tx_index: 0 };
        if let Err(e) = self.rewind_tx.send(signal).await {
            warn!(
                target: LOG_TARGET,
                error = %e,
                "Failed to send rewind signal; DB checkpoint deleted, will be picked up on next start.",
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use katana_provider::DbProviderFactory;
    use tokio::sync::mpsc;

    use super::*;

    fn setup() -> (MessagingController<DbProviderFactory>, mpsc::Receiver<RewindSignal>) {
        let provider = DbProviderFactory::new_in_memory();
        let (tx, rx) = mpsc::channel(1);
        (MessagingController::new(provider, 7, tx), rx)
    }

    #[tokio::test]
    async fn get_checkpoint_returns_none_when_absent() {
        let (controller, _rx) = setup();
        let cp = controller.get_checkpoint().unwrap();
        assert!(cp.is_none());
    }

    #[tokio::test]
    async fn set_checkpoint_persists_value_and_emits_rewind_signal_at_tx_index_plus_one() {
        let (controller, mut rx) = setup();

        controller.set_checkpoint(100, 5).await.unwrap();

        let cp = controller.get_checkpoint().unwrap().expect("checkpoint persisted");
        assert_eq!(cp.block, 100);
        assert_eq!(cp.tx_index, 5);

        let signal = rx.try_recv().expect("rewind signal sent");
        assert_eq!(signal.from_block, 100);
        // The DB checkpoint records the last *processed* message; the live
        // cursor must resume one past it.
        assert_eq!(signal.from_tx_index, 6);
    }

    #[tokio::test]
    async fn reset_checkpoint_deletes_row_and_emits_default_from_block_signal() {
        let (controller, mut rx) = setup();

        controller.set_checkpoint(42, 9).await.unwrap();
        let _ = rx.try_recv().expect("set signal");

        controller.reset_checkpoint().await.unwrap();

        let cp = controller.get_checkpoint().unwrap();
        assert!(cp.is_none(), "row deleted");

        let signal = rx.try_recv().expect("reset signal sent");
        assert_eq!(signal.from_block, 7, "default_from_block snapshotted at construction");
        assert_eq!(signal.from_tx_index, 0);
    }

    /// A failed channel send must not fail the call — the DB write is the
    /// source of truth and the next start picks up the value.
    #[tokio::test]
    async fn set_and_reset_succeed_when_receiver_dropped() {
        let provider = DbProviderFactory::new_in_memory();
        let (tx, rx) = mpsc::channel(1);
        drop(rx);
        let controller = MessagingController::new(provider, 0, tx);

        controller.set_checkpoint(1, 2).await.expect("set succeeds with dropped receiver");
        controller.reset_checkpoint().await.expect("reset succeeds with dropped receiver");
    }
}
