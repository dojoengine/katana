use std::sync::Arc;

use futures::StreamExt;
use katana_pool::api::TransactionPool;
use katana_pool::TxPool;
use katana_primitives::transaction::{ExecutableTxWithHash, L1HandlerTx, TxHash};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::{MessagingOutcome, Messenger, LOG_TARGET};

impl std::fmt::Debug for MessagingServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessagingServer").finish_non_exhaustive()
    }
}

/// Checkpoint data persisted after each successfully processed message.
///
/// Identifies the last fully processed message so a restart can resume from the next
/// position.
#[derive(Debug, Clone)]
pub struct Checkpoint {
    /// The settlement chain block the message was emitted in.
    pub block: u64,
    /// The transaction index of the message within `block`.
    pub tx_index: u64,
}

/// A record of a settlement chain L1 transaction successfully spawning an L2
/// L1Handler transaction. Used to populate the L1->L2 index that powers
/// `starknet_getMessagesStatus`.
#[derive(Debug, Clone)]
pub struct L1ToL2Record {
    /// Settlement chain transaction hash that emitted the originating event/log.
    pub l1_tx_hash: [u8; 32],
    /// L2 L1Handler transaction hash.
    pub l2_tx_hash: TxHash,
}

/// Combined record passed to [`OnCommitCallback`] after a successful pool insert.
///
/// The callback must persist both fields **atomically** (i.e. in a single
/// storage transaction) so the index entry and the checkpoint advance together.
/// If they could fail independently, an index write that succeeds but a checkpoint
/// that fails (or vice-versa) would let restart skip the message while losing the
/// L1->L2 mapping forever, breaking `starknet_getMessagesStatus`.
#[derive(Debug, Clone)]
pub struct MessageCommit {
    /// L1->L2 mapping for the `starknet_getMessagesStatus` index.
    pub record: L1ToL2Record,
    /// Checkpoint to advance to after this message is fully processed.
    pub checkpoint: Checkpoint,
}

/// Callback invoked by the server after each successful pool insert. The callback
/// receives both the L1->L2 mapping and the new checkpoint, and is expected to
/// persist them atomically.
///
/// If the callback returns `Err`, the server logs the failure and stops processing
/// the current batch. The in-memory stream cursor has already advanced past the
/// gather range, so on restart the messenger will re-gather and re-attempt; the
/// pool's hash-level dedup prevents duplicate L1Handler txs.
pub type OnCommitCallback =
    Arc<dyn Fn(MessageCommit) -> Result<(), anyhow::Error> + Send + Sync>;

/// The messaging server drains a [`Messenger`] stream, adds gathered transactions
/// to the transaction pool, and persists checkpoints + L1->L2 index entries.
pub struct MessagingServer {
    messenger: Box<dyn Messenger>,
    pool: Option<TxPool>,
    on_commit: Option<OnCommitCallback>,
}

impl MessagingServer {
    /// Create a new messaging server wrapping the given messenger.
    pub fn new(messenger: Box<dyn Messenger>) -> Self {
        Self { messenger, pool: None, on_commit: None }
    }

    /// Set the transaction pool where gathered L1Handler transactions will be added.
    pub fn pool(mut self, pool: TxPool) -> Self {
        self.pool = Some(pool);
        self
    }

    /// Set the per-message commit callback. The callback fires after each successful
    /// pool insert and is responsible for atomically persisting:
    /// - The L1->L2 index entry for `starknet_getMessagesStatus`
    /// - The checkpoint, so a restart resumes from the next position
    pub fn on_commit(mut self, callback: OnCommitCallback) -> Self {
        self.on_commit = Some(callback);
        self
    }

    /// Start the messaging server. Returns a handle for lifecycle control.
    ///
    /// The server runs a background task that:
    /// 1. Drains the messenger stream for positioned messages
    /// 2. Adds each message to the pool individually
    /// 3. On each successful insert, invokes `on_commit` to atomically persist the
    ///    L1->L2 index entry and the checkpoint — enabling fine-grained resume after
    ///    a crash without losing index entries
    pub fn start(self) -> MessagingHandle {
        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        let pool = self.pool.expect("pool must be set before starting");
        let mut messenger = self.messenger;
        let on_commit = self.on_commit;

        let task_handle = tokio::spawn(async move {
            tokio::pin!(let shutdown = shutdown_rx;);

            loop {
                tokio::select! {
                    outcome = messenger.next() => {
                        match outcome {
                            Some(MessagingOutcome { settlement_block, messages }) => {
                                let total = messages.len();
                                let mut inserted: usize = 0;

                                for msg in messages {
                                    let hash = msg.tx.calculate_hash();
                                    trace_l1_handler_tx_exec(hash, &msg.tx);

                                    let insert_result = pool
                                        .add_transaction(ExecutableTxWithHash {
                                            hash,
                                            transaction: msg.tx.into(),
                                        })
                                        .await;

                                    match insert_result {
                                        Ok(_) => {
                                            inserted += 1;
                                            // Persist the index entry and the checkpoint
                                            // atomically. If the commit fails, abort the rest
                                            // of the batch — the cursor has already advanced,
                                            // so on restart we'll re-gather and re-attempt.
                                            if let Some(ref cb) = on_commit {
                                                let commit = MessageCommit {
                                                    record: L1ToL2Record {
                                                        l1_tx_hash: msg.l1_tx_hash,
                                                        l2_tx_hash: hash,
                                                    },
                                                    checkpoint: Checkpoint {
                                                        block: msg.block,
                                                        tx_index: msg.tx_index,
                                                    },
                                                };
                                                if let Err(e) = cb(commit) {
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
                                        total,
                                        %settlement_block,
                                        "Collected messages from settlement chain.",
                                    );
                                }
                            }
                            None => {
                                // Stream ended
                                break;
                            }
                        }
                    }
                    _ = &mut shutdown => {
                        break;
                    }
                }
            }
        });

        MessagingHandle { shutdown_tx: Some(shutdown_tx), task_handle }
    }
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

fn trace_l1_handler_tx_exec(hash: TxHash, tx: &L1HandlerTx) {
    let calldata_str: Vec<_> = tx.calldata.iter().map(|f| format!("{f:#x}")).collect();

    #[rustfmt::skip]
    info!(
        target: LOG_TARGET,
        tx_hash = %format!("{:#x}", hash),
        contract_address = %tx.contract_address,
        selector = %format!("{:#x}", tx.entry_point_selector),
        calldata = %calldata_str.join(", "),
        "L1Handler transaction added to the pool.",
    );
}
