use futures::StreamExt;
use katana_pool::api::TransactionPool;
use katana_pool::TxPool;
use katana_primitives::transaction::{ExecutableTxWithHash, L1HandlerTx, TxHash};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::{Messenger, MessagingOutcome, LOG_TARGET};

impl std::fmt::Debug for MessagingServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessagingServer").finish_non_exhaustive()
    }
}

/// Checkpoint data passed to the on_gather callback.
///
/// Identifies the last fully processed message so a restart can resume from the next
/// position. The callback is invoked once per successfully pool-inserted message.
#[derive(Debug, Clone)]
pub struct Checkpoint {
    /// The settlement chain block the message was emitted in.
    pub block: u64,
    /// The transaction index of the message within `block`.
    pub tx_index: u64,
}

/// Callback invoked by the server after each successful pool insert to persist
/// progress. Caller is responsible for any durability guarantees (e.g., a DB commit).
pub type OnGatherCallback = Box<dyn Fn(Checkpoint) + Send + Sync>;

/// The messaging server drains a [`Messenger`] stream, adds gathered transactions
/// to the transaction pool, and persists checkpoints.
pub struct MessagingServer {
    messenger: Box<dyn Messenger>,
    pool: Option<TxPool>,
    on_gather: Option<OnGatherCallback>,
}

impl MessagingServer {
    /// Create a new messaging server wrapping the given messenger.
    pub fn new(messenger: Box<dyn Messenger>) -> Self {
        Self { messenger, pool: None, on_gather: None }
    }

    /// Set the transaction pool where gathered L1Handler transactions will be added.
    pub fn pool(mut self, pool: TxPool) -> Self {
        self.pool = Some(pool);
        self
    }

    /// Set a callback invoked after each successful pool insert with the checkpoint
    /// of the just-processed message.
    pub fn on_gather(mut self, callback: OnGatherCallback) -> Self {
        self.on_gather = Some(callback);
        self
    }

    /// Start the messaging server. Returns a handle for lifecycle control.
    ///
    /// The server runs a background task that:
    /// 1. Drains the messenger stream for positioned messages
    /// 2. Adds each message to the pool individually
    /// 3. On each successful insert, invokes `on_gather` with the message's checkpoint
    ///    — enabling fine-grained resume after a crash
    pub fn start(self) -> MessagingHandle {
        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        let pool = self.pool.expect("pool must be set before starting");
        let mut messenger = self.messenger;
        let on_gather = self.on_gather;

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
                                            // Checkpoint AFTER a successful insert only.
                                            // On pool failure we do not advance, so the next
                                            // gather re-attempts this message.
                                            if let Some(ref cb) = on_gather {
                                                cb(Checkpoint { block: msg.block, tx_index: msg.tx_index });
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
