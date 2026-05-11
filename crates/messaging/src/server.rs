use anyhow::Context;
use futures::StreamExt;
use katana_pool::api::TransactionPool;
use katana_pool::TxPool;
use katana_primitives::chain::ChainId;
use katana_primitives::transaction::{ExecutableTxWithHash, TxHash};
use katana_provider::api::messaging::{
    MessagingCheckpoint, MessagingCheckpointProvider, MessagingL1ToL2IndexWriter,
};
use katana_provider::{MutableProvider, ProviderFactory, ProviderRW};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::{build_messenger, MessagingConfig, MessagingOutcome, LOG_TARGET};

/// Identifier used to namespace the persisted messaging checkpoint within the
/// shared `MessagingCheckpoints` table.
const CHECKPOINT_ID: &str = "messaging";

/// The messaging server.
///
/// A "static" node component held on `Node` whether or not messaging is enabled.
/// [`Self::start`] is non-consuming and mirrors `RpcServer::start`. When `config`
/// is `None` (messaging disabled), [`Self::start`] returns `Ok(None)` — no task is
/// spawned. Otherwise it reads the resume checkpoint, builds the messenger, and
/// spawns the drain loop.
///
/// The server depends directly on the provider factory `P` for both reading the
/// resume checkpoint at boot and atomically persisting the L1->L2 index entry +
/// checkpoint after each successful pool insert.
pub struct MessagingServer<P> {
    config: Option<MessagingConfig>,
    chain_id: ChainId,
    pool: TxPool,
    provider: P,
}

impl<P> MessagingServer<P> {
    /// Create a new messaging server.
    ///
    /// `config == None` makes [`Self::start`] a no-op (returns `Ok(None)`).
    pub fn new(
        config: Option<MessagingConfig>,
        chain_id: ChainId,
        pool: TxPool,
        provider: P,
    ) -> Self {
        Self { config, chain_id, pool, provider }
    }
}

impl<P> MessagingServer<P>
where
    P: ProviderFactory + Clone + Send + Sync + 'static,
    <P as ProviderFactory>::ProviderMut:
        ProviderRW + MessagingCheckpointProvider + MessagingL1ToL2IndexWriter + MutableProvider,
{
    /// Start the messaging server.
    ///
    /// Returns `Ok(None)` when messaging is disabled (no config) — the component
    /// exists but no task is spawned. Returns `Ok(Some(handle))` after successfully
    /// reading the checkpoint, building the messenger, and spawning the drain loop.
    pub fn start(&self) -> Result<Option<MessagingHandle>, anyhow::Error> {
        let Some(config) = &self.config else {
            return Ok(None);
        };

        let (from_block, from_tx_index) = self.resume_cursor(config)?;
        let mut messenger = build_messenger(config, self.chain_id, from_block, from_tx_index)?;

        let pool = self.pool.clone();
        let provider = self.provider.clone();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        let task_handle = tokio::spawn(async move {
            tokio::pin!(let shutdown = shutdown_rx;);

            loop {
                tokio::select! {
                    outcome = messenger.next() => {
                        match outcome {
                            None => break, // Stream ended

                            Some(MessagingOutcome { settlement_block, messages }) => {
                                let total_messages = messages.len();
                                let mut inserted: usize = 0;

                                for msg in messages {
                                    let hash = msg.tx.calculate_hash();
                                    info!(target: LOG_TARGET, tx_hash = %format!("{:#x}", hash), "L1Handler transaction added to the pool.");

                                    let pool_tx = ExecutableTxWithHash { hash, transaction: msg.tx.into() };
                                    let insert_result = pool.add_transaction(pool_tx).await;

                                    match insert_result {
                                        Ok(_) => {
                                            inserted += 1;

                                            // Atomically persist the L1->L2 index entry and the
                                            // checkpoint in a single DB transaction. If either
                                            // write or the commit fails, NEITHER is persisted —
                                            // on restart we'll re-gather and re-attempt. Splitting
                                            // these previously meant a failed index write paired
                                            // with a successful checkpoint write would silently
                                            // drop the L1->L2 mapping forever.
                                            if let Err(e) = commit_message(
                                                &provider,
                                                &msg.l1_tx_hash,
                                                hash,
                                                msg.block,
                                                msg.tx_index,
                                            ) {
                                                warn!(
                                                    target: LOG_TARGET,
                                                    error = %e,
                                                    block = msg.block,
                                                    tx_index = msg.tx_index,
                                                    tx_hash = %format!("{hash:#x}"),
                                                    "Failed to commit messaging state; aborting batch, will retry on next gather.",
                                                );
                                                break;
                                            }
                                        }
                                        Err(e) => {
                                            warn!(
                                                target: LOG_TARGET,
                                                error = %e,
                                                block = msg.block,
                                                tx_index = msg.tx_index,
                                                tx_hash = %format!("{hash:#x}"),
                                                "Failed to add L1Handler transaction to pool; will retry on next gather.",
                                            );

                                            // Stop processing this batch. The stream's cursor
                                            // was already advanced past the current gather range;
                                            // the retry for this message will rely on the pool's
                                            // hash-level deduplication of successful inserts and
                                            // re-gather on the next tick.
                                            break;
                                        }
                                    }
                                }

                                if inserted > 0 {
                                    info!(
                                        target: LOG_TARGET,
                                        inserted,
                                        total_messages,
                                        %settlement_block,
                                        "Collected messages from settlement chain.",
                                    );
                                }
                            }
                        }
                    }

                    _ = &mut shutdown => {
                        break;
                    }
                }
            }
        });

        Ok(Some(MessagingHandle { shutdown_tx: Some(shutdown_tx), task_handle }))
    }

    /// Determine `(from_block, from_tx_index)` for the next gather.
    ///
    /// If a persisted checkpoint exists, resume from the message immediately after
    /// it. Otherwise fall back to `config.from_block`.
    fn resume_cursor(&self, config: &MessagingConfig) -> Result<(u64, u64), anyhow::Error> {
        let db_tx = self.provider.provider_mut();
        let cp = db_tx.messaging_checkpoint(CHECKPOINT_ID).context("read messaging checkpoint")?;
        db_tx.commit().context("commit checkpoint read tx")?;

        let checkpoint = match cp {
            Some(c) => (c.block, c.tx_index + 1),
            None => (config.from_block, 0),
        };

        Ok(checkpoint)
    }
}

impl<P> std::fmt::Debug for MessagingServer<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessagingServer").finish_non_exhaustive()
    }
}

impl<P: Clone> Clone for MessagingServer<P> {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            chain_id: self.chain_id,
            pool: self.pool.clone(),
            provider: self.provider.clone(),
        }
    }
}

/// Atomically record the L1->L2 mapping and advance the checkpoint inside a single
/// DB transaction. Returns an error if any of the staged writes or the commit fail.
fn commit_message<P>(
    provider: &P,
    l1_tx_hash: &[u8; 32],
    l2_tx_hash: TxHash,
    block: u64,
    tx_index: u64,
) -> Result<(), anyhow::Error>
where
    P: ProviderFactory,
    <P as ProviderFactory>::ProviderMut:
        MessagingCheckpointProvider + MessagingL1ToL2IndexWriter + MutableProvider,
{
    let db_tx = provider.provider_mut();
    db_tx.record_l1_to_l2(l1_tx_hash, l2_tx_hash)?;
    db_tx.set_messaging_checkpoint(CHECKPOINT_ID, &MessagingCheckpoint { block, tx_index })?;
    db_tx.commit()?;
    Ok(())
}

/// Handle to a running messaging server, providing lifecycle control.
pub struct MessagingHandle {
    shutdown_tx: Option<oneshot::Sender<()>>,
    task_handle: JoinHandle<()>,
}

impl std::fmt::Debug for MessagingHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessagingHandle").finish_non_exhaustive()
    }
}

impl MessagingHandle {
    /// Signal the messaging server to stop.
    pub fn stop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }

    /// Wait until the messaging server has fully stopped.
    pub async fn stopped(self) {
        let _ = self.task_handle.await;
    }
}
