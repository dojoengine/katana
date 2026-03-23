use futures::StreamExt;
use katana_pool::{TransactionPool, TxPool};
use katana_primitives::transaction::{ExecutableTxWithHash, L1HandlerTx, TxHash};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tracing::info;

use crate::{Messenger, MessagingOutcome, LOG_TARGET};

impl std::fmt::Debug for MessagingServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessagingServer").finish_non_exhaustive()
    }
}

/// Checkpoint data passed to the on_gather callback.
#[derive(Debug, Clone)]
pub struct Checkpoint {
    /// The settlement chain block number last processed.
    pub block: u64,
    /// The transaction index within `block` up to which messages were processed.
    pub tx_index: u64,
}

/// Callback invoked by the server after each successful message gather.
/// Used for checkpointing and any other post-gather side effects.
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

    /// Set a callback to be invoked after each successful gather with the latest
    /// settlement block number. Typically used to persist the checkpoint.
    pub fn on_gather(mut self, callback: OnGatherCallback) -> Self {
        self.on_gather = Some(callback);
        self
    }

    /// Start the messaging server. Returns a handle for lifecycle control.
    ///
    /// The server runs a background task that:
    /// 1. Drains the messenger stream
    /// 2. Adds gathered transactions to the pool
    /// 3. Invokes the on_gather callback (for checkpointing)
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
                            Some(MessagingOutcome { settlement_block, tx_index, transactions }) => {
                                let msg_count = transactions.len();

                                for tx in transactions {
                                    let hash = tx.calculate_hash();
                                    trace_l1_handler_tx_exec(hash, &tx);
                                    let _ = pool
                                        .add_transaction(ExecutableTxWithHash {
                                            hash,
                                            transaction: tx.into(),
                                        })
                                        .await;
                                }

                                if msg_count > 0 {
                                    info!(
                                        target: LOG_TARGET,
                                        %msg_count,
                                        %settlement_block,
                                        "Collected messages from settlement chain."
                                    );
                                }

                                // Persist checkpoint
                                if let Some(ref cb) = on_gather {
                                    cb(Checkpoint { block: settlement_block, tx_index });
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
