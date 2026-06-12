use std::sync::Mutex;

use anyhow::{anyhow, Context};
use futures::StreamExt;
use katana_pool::api::TransactionPool;
use katana_pool::TxPool;
use katana_primitives::chain::ChainId;
use katana_primitives::transaction::{ExecutableTxWithHash, TxHash};
use katana_provider::api::messaging::{
    MessagingCheckpoint, MessagingCheckpointProvider, MessagingL1ToL2IndexWriter,
};
use katana_provider::{MutableProvider, ProviderFactory, ProviderRW};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::controller::{MessagingController, RewindSignal};
use crate::stream::collector::ethereum::EthereumCollector;
use crate::stream::collector::starknet::StarknetCollector;
use crate::stream::trigger::IntervalTrigger;
use crate::stream::MessageStream;
use crate::{MessagingOutcome, Messenger, SettlementChainConfig, LOG_TARGET};

/// Default poll interval (in seconds) between gather ticks.
const DEFAULT_INTERVAL: u64 = 2;

/// The messaging service.
///
/// [`Self::start`] is non-consuming and mirrors `RpcServer::start`. It reads the
/// resume checkpoint, builds the messenger, and spawns the drain loop. Calling
/// [`start`](Self::start) more than once returns an error: the rewind receiver
/// is single-consumer and is taken on the first call.
///
/// The service depends directly on the provider factory `P` for both reading
/// the resume checkpoint at boot and atomically persisting the L1->L2 index
/// entry + checkpoint after each successful pool insert.
///
/// Configure with builder-style setters
/// ([`interval`](Self::interval), [`from_block`](Self::from_block),
/// [`confirmation_depth`](Self::confirmation_depth)) before calling
/// [`start`](Self::start). The settlement chain is required at construction.
pub struct MessagingService<P, Pl = TxPool> {
    chain_id: ChainId,
    pool: Pl,
    provider: P,

    settlement: SettlementChainConfig,
    interval: u64,
    from_block: u64,
    confirmation_depth: u64,

    /// Sender shared with all controllers; cloning is cheap.
    rewind_tx: mpsc::Sender<RewindSignal>,
    /// Owned by the service until [`start`](Self::start) takes it. Clones of the
    /// service get an empty mutex so only the original can drive the drain loop.
    /// Wrapped in a `Mutex` because `start` is `&self`.
    rewind_rx: Mutex<Option<mpsc::Receiver<RewindSignal>>>,
}

impl<P, Pl> MessagingService<P, Pl> {
    /// Create a new messaging service for the given settlement chain.
    pub fn new(
        chain_id: ChainId,
        pool: Pl,
        provider: P,
        settlement: SettlementChainConfig,
    ) -> Self {
        // Capacity 1 with `send().await` gives natural back-pressure on rapid
        // rewinds; operator-issued resets don't burst.
        let (rewind_tx, rewind_rx) = mpsc::channel(1);
        Self {
            chain_id,
            pool,
            provider,
            settlement,
            interval: DEFAULT_INTERVAL,
            from_block: 0,
            confirmation_depth: 0,
            rewind_tx,
            rewind_rx: Mutex::new(Some(rewind_rx)),
        }
    }

    /// Set the interval, in seconds, at which the messaging service polls the
    /// settlement chain for new blocks. Default is `2` seconds.
    pub fn interval(mut self, interval: u64) -> Self {
        self.interval = interval;
        self
    }

    /// Set the settlement-chain block from which to start gathering on a fresh
    /// run (no persisted checkpoint). Default is `0`.
    pub fn from_block(mut self, from_block: u64) -> Self {
        self.from_block = from_block;
        self
    }

    /// Set the number of settlement-chain confirmations required before a block
    /// is considered safe to gather from. Default is `0` (no protection).
    pub fn confirmation_depth(mut self, confirmation_depth: u64) -> Self {
        self.confirmation_depth = confirmation_depth;
        self
    }
}

impl<P, Pl> MessagingService<P, Pl>
where
    P: ProviderFactory + Clone + Send + Sync + 'static,
    <P as ProviderFactory>::ProviderMut:
        ProviderRW + MessagingCheckpointProvider + MessagingL1ToL2IndexWriter + MutableProvider,
    Pl: TransactionPool<Transaction = ExecutableTxWithHash> + Clone + Send + Sync + 'static,
{
    /// Returns a [`MessagingController`] that can read/write the persisted
    /// checkpoint and signal a live rewind to a running drain task.
    ///
    /// Snapshots the configured `from_block` as the controller's default — call
    /// [`from_block`](Self::from_block) before this.
    pub fn controller(&self) -> MessagingController<P>
    where
        P: Clone,
    {
        MessagingController::new(self.provider.clone(), self.from_block, self.rewind_tx.clone())
    }

    /// Start the messaging service.
    ///
    /// Reads the resume checkpoint, builds the messenger, and spawns the drain
    /// loop. Returns an error if `start()` has already been called on this
    /// instance (the rewind receiver is single-consumer).
    pub fn start(&self) -> Result<MessagingServiceHandle, anyhow::Error> {
        let mut rewind_rx = self
            .rewind_rx
            .lock()
            .map_err(|_| anyhow!("rewind receiver mutex poisoned"))?
            .take()
            .ok_or_else(|| anyhow!("messaging service already started"))?;

        let (from_block, from_tx_index) = resume_cursor(&self.provider, self.from_block)?;

        let trigger = IntervalTrigger::new(self.interval);

        let mut messenger: Box<dyn Messenger> = match &self.settlement {
            SettlementChainConfig::Ethereum { rpc_url, contract_address } => {
                let collector = EthereumCollector::new(rpc_url.clone(), *contract_address)?;
                Box::new(MessageStream::with_cursor(
                    collector,
                    trigger,
                    self.chain_id,
                    from_block,
                    from_tx_index,
                    self.confirmation_depth,
                ))
            }
            SettlementChainConfig::Starknet { rpc_url, contract_address } => {
                let collector = StarknetCollector::new(rpc_url.clone(), *contract_address)?;
                Box::new(MessageStream::with_cursor(
                    collector,
                    trigger,
                    self.chain_id,
                    from_block,
                    from_tx_index,
                    self.confirmation_depth,
                ))
            }
        };

        let pool = self.pool.clone();
        let provider = self.provider.clone();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        let task_handle = tokio::spawn(async move {
            tokio::pin!(let shutdown = shutdown_rx;);

            loop {
                tokio::select! {
                    // Shutdown takes priority over both gather and rewind so
                    // a stop signal can't be starved by busy work.
                    biased;

                    _ = &mut shutdown => {
                        break;
                    }

                    Some(sig) = rewind_rx.recv() => {
                        info!(
                            target: LOG_TARGET,
                            from_block = sig.from_block,
                            from_tx_index = sig.from_tx_index,
                            "Rewinding messenger cursor.",
                        );
                        messenger.rewind(sig.from_block, sig.from_tx_index);
                    }

                    outcome = messenger.next() => {
                        match outcome {
                            None => break, // Stream ended

                            Some(MessagingOutcome { settlement_block, messages }) => {
                                let total_messages = messages.len();
                                let mut inserted: usize = 0;

                                for msg in messages {
                                    let hash = msg.tx.calculate_hash();
                                    info!(target: LOG_TARGET, tx_hash = %format!("{:#x}", hash), msg_hash = %msg.tx.message_hash, "L1Handler transaction added to the pool.");

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
                                            if let Err(error) = commit_message(
                                                &provider,
                                                &msg.l1_tx_hash,
                                                hash,
                                                msg.block,
                                                msg.tx_index,
                                            ) {
                                                warn!(
                                                    target: LOG_TARGET,
                                                    %error,
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
                }
            }
        });

        info!(target: LOG_TARGET, "Messaging service started.");

        Ok(MessagingServiceHandle { shutdown_tx: Some(shutdown_tx), task_handle })
    }
}

/// Determine `(from_block, from_tx_index)` for the next gather.
///
/// If a persisted checkpoint exists, resume from the message immediately after it
/// (same block, next `tx_index`). Otherwise fall back to `default_from_block`.
fn resume_cursor<P>(provider: &P, default_from_block: u64) -> Result<(u64, u64), anyhow::Error>
where
    P: ProviderFactory,
    <P as ProviderFactory>::ProviderMut: MessagingCheckpointProvider + MutableProvider,
{
    let db_tx = provider.provider_mut();
    let cp = db_tx.messaging_checkpoint().context("read messaging checkpoint")?;
    db_tx.commit().context("commit checkpoint read tx")?;

    Ok(match cp {
        Some(c) => (c.block, c.tx_index + 1),
        None => (default_from_block, 0),
    })
}

impl<P, Pl> std::fmt::Debug for MessagingService<P, Pl> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessagingService").finish_non_exhaustive()
    }
}

impl<P: Clone, Pl: Clone> Clone for MessagingService<P, Pl> {
    fn clone(&self) -> Self {
        Self {
            chain_id: self.chain_id,
            pool: self.pool.clone(),
            provider: self.provider.clone(),
            settlement: self.settlement.clone(),
            interval: self.interval,
            from_block: self.from_block,
            confirmation_depth: self.confirmation_depth,
            rewind_tx: self.rewind_tx.clone(),
            // Clones share the sender but cannot be started — the receiver is
            // not cloneable and only the original service can drive the drain
            // loop.
            rewind_rx: Mutex::new(None),
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
    db_tx.set_messaging_checkpoint(&MessagingCheckpoint { block, tx_index })?;
    db_tx.commit()?;
    Ok(())
}

/// Handle to a running messaging server, providing lifecycle control.
pub struct MessagingServiceHandle {
    shutdown_tx: Option<oneshot::Sender<()>>,
    task_handle: JoinHandle<()>,
}

impl std::fmt::Debug for MessagingServiceHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessagingServiceHandle").finish_non_exhaustive()
    }
}

impl MessagingServiceHandle {
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use katana_pool::ordering::FiFo;
    use katana_pool::pool::Pool;
    use katana_pool::validation::NoopValidator;
    use katana_primitives::transaction::ExecutableTxWithHash;
    use katana_primitives::Felt;
    use katana_provider::api::messaging::MessagingL1ToL2IndexProvider;
    use katana_provider::DbProviderFactory;
    use url::Url;

    use super::*;

    /// No-op pool used by the lifecycle tests. The drain task never actually inserts
    /// transactions in these tests (the configured settlement endpoint is unroutable),
    /// so this type only needs to satisfy the trait bounds of `start()`.
    type NoopPool =
        Pool<ExecutableTxWithHash, NoopValidator<ExecutableTxWithHash>, FiFo<ExecutableTxWithHash>>;

    fn noop_pool() -> NoopPool {
        Pool::new(NoopValidator::new(), FiFo::new())
    }

    /// Settlement config pointing at a non-routable URL. The drain task may try
    /// `latest_block()` against this; it'll fail or pend, which is fine — the
    /// lifecycle tests don't depend on any successful gather.
    fn unroutable_settlement() -> SettlementChainConfig {
        SettlementChainConfig::Ethereum {
            rpc_url: Url::parse("http://127.0.0.1:1/").unwrap(),
            contract_address: Default::default(),
        }
    }

    #[test]
    fn resume_cursor_falls_back_to_default_from_block_when_no_checkpoint_persisted() {
        let provider = DbProviderFactory::new_in_memory();

        let (from_block, from_tx_index) = resume_cursor(&provider, 42).unwrap();

        assert_eq!(from_block, 42, "should use the default from_block on fresh start");
        assert_eq!(from_tx_index, 0, "should start from the first tx in the block");
    }

    #[test]
    fn resume_cursor_resumes_on_same_block_at_next_tx_index_after_a_persisted_checkpoint() {
        let provider = DbProviderFactory::new_in_memory();

        // Persist a checkpoint marking message at (block=100, tx_index=5) as fully processed.
        let db_tx = provider.provider_mut();
        db_tx.set_messaging_checkpoint(&MessagingCheckpoint { block: 100, tx_index: 5 }).unwrap();
        db_tx.commit().unwrap();

        // The default from_block is intentionally far below the persisted checkpoint —
        // the persisted state must take precedence to avoid re-processing on restart.
        let (from_block, from_tx_index) = resume_cursor(&provider, 0).unwrap();

        assert_eq!(from_block, 100, "should resume on the same block as the checkpoint");
        assert_eq!(
            from_tx_index, 6,
            "should resume at tx_index + 1 (the message after the last processed one)"
        );
    }

    /// One pool insert advances both the L1->L2 index AND the checkpoint in a single
    /// DB transaction. Asserting on both after one call to `commit_message` ensures
    /// the atomicity contract is preserved.
    #[test]
    fn commit_message_persists_both_index_entry_and_checkpoint() {
        let provider = DbProviderFactory::new_in_memory();
        let l1 = [7u8; 32];
        let l2 = Felt::from(0xdead_beef_u64);

        commit_message(&provider, &l1, l2, 10, 3).unwrap();

        let db_tx = provider.provider_mut();
        let mapped = db_tx.l2_txs_for_l1(&l1).unwrap();
        let cp = db_tx.messaging_checkpoint().unwrap().expect("checkpoint should exist");
        db_tx.commit().unwrap();

        assert_eq!(mapped, vec![l2], "L1->L2 index entry should be written");
        assert_eq!(cp.block, 10);
        assert_eq!(cp.tx_index, 3);
    }

    /// A single L1 transaction can emit multiple `LogMessageToL2` events, each spawning
    /// its own L2 L1Handler. The DupSort table must hold all of them under the same key.
    #[test]
    fn commit_message_records_multiple_l2_txs_for_the_same_l1_tx() {
        let provider = DbProviderFactory::new_in_memory();
        let l1 = [9u8; 32];
        let l2_a = Felt::from(1u64);
        let l2_b = Felt::from(2u64);

        commit_message(&provider, &l1, l2_a, 0, 0).unwrap();
        commit_message(&provider, &l1, l2_b, 0, 1).unwrap();

        let db_tx = provider.provider_mut();
        let mapped = db_tx.l2_txs_for_l1(&l1).unwrap();
        db_tx.commit().unwrap();

        assert_eq!(mapped.len(), 2, "both L1->L2 entries should be present");
        assert!(mapped.contains(&l2_a));
        assert!(mapped.contains(&l2_b));
    }

    /// DupSort put on the same `(key, value)` pair is a silent no-op — required for
    /// the re-gather-and-retry recovery path (after a pool insert succeeds but a
    /// subsequent message in the batch fails) to be idempotent.
    #[test]
    fn commit_message_is_idempotent_for_the_same_l1_l2_pair() {
        let provider = DbProviderFactory::new_in_memory();
        let l1 = [1u8; 32];
        let l2 = Felt::from(42u64);

        commit_message(&provider, &l1, l2, 0, 0).unwrap();
        commit_message(&provider, &l1, l2, 1, 1).unwrap();

        let db_tx = provider.provider_mut();
        let mapped = db_tx.l2_txs_for_l1(&l1).unwrap();
        db_tx.commit().unwrap();

        assert_eq!(mapped.len(), 1, "same (l1, l2) pair should not duplicate");
    }

    /// Successive commits monotonically advance the checkpoint to the latest message.
    /// This is what enables fine-grained per-message resume.
    #[test]
    fn commit_message_advances_checkpoint_monotonically() {
        let provider = DbProviderFactory::new_in_memory();
        let l1 = [3u8; 32];
        let l2 = Felt::from(99u64);

        commit_message(&provider, &l1, l2, 5, 0).unwrap();
        commit_message(&provider, &l1, l2, 5, 7).unwrap();
        commit_message(&provider, &l1, l2, 10, 2).unwrap();

        let db_tx = provider.provider_mut();
        let cp = db_tx.messaging_checkpoint().unwrap().expect("checkpoint");
        db_tx.commit().unwrap();

        assert_eq!(cp.block, 10, "checkpoint should reflect the latest committed message");
        assert_eq!(cp.tx_index, 2);
    }

    // -------------------------------------------------------------------------
    // Lifecycle tests
    //
    // These exercise `MessagingService::start` itself — that the rewind_rx is
    // single-take, that clones can't be started, and that a closed rewind
    // channel doesn't kill the drain task. They use a non-routable settlement
    // endpoint; the drain task never produces work but stays alive, which is
    // all the lifecycle invariants require.
    // -------------------------------------------------------------------------

    /// The `rewind_rx` is taken on first `start()`; a second call on the same
    /// instance must fail with a clear "already started" error rather than
    /// silently spawning a second drain task that competes for rewind signals.
    #[tokio::test]
    async fn start_twice_returns_error() {
        let provider = DbProviderFactory::new_in_memory();
        let pool = noop_pool();
        let server =
            MessagingService::new(ChainId::default(), pool, provider, unroutable_settlement())
                .interval(60);

        let mut handle = server.start().expect("first start succeeds");

        let err = server.start().expect_err("second start must fail");
        let msg = err.to_string();
        assert!(msg.contains("already started"), "expected 'already started' in error, got: {msg}");

        // Clean up the first task so the test process exits cleanly.
        handle.stop();
        handle.stopped().await;
    }

    /// `Clone for MessagingService` deliberately sets `rewind_rx: None` on the
    /// clone so only the original instance can drive the drain loop. Starting
    /// a clone must fail with the same error as a double-start.
    #[tokio::test]
    async fn clone_cannot_be_started() {
        let provider = DbProviderFactory::new_in_memory();
        let pool = noop_pool();
        let server =
            MessagingService::new(ChainId::default(), pool, provider, unroutable_settlement())
                .interval(60);

        let clone = server.clone();

        let err = clone.start().expect_err("starting a clone must fail");
        let msg = err.to_string();
        assert!(msg.contains("already started"), "expected 'already started' in error, got: {msg}");

        // The original is still startable (rewind_rx wasn't taken from it).
        let mut handle = server.start().expect("original is still startable after cloning");
        handle.stop();
        handle.stopped().await;
    }

    /// Dropping the controller (and hence one rewind_tx sender) must not kill
    /// the running drain task. The other arms of the `select!` (shutdown,
    /// messenger.next) keep firing; the rewind arm just goes permanently
    /// inactive once all senders are gone. This guards against a regression
    /// where the loop would exit on `rewind_rx.recv() == None`.
    #[tokio::test]
    async fn rewind_sender_dropped_does_not_kill_task() {
        let provider = DbProviderFactory::new_in_memory();
        let pool = noop_pool();
        let server =
            MessagingService::new(ChainId::default(), pool, provider, unroutable_settlement())
                .interval(60);

        let controller = server.controller();
        let mut handle = server.start().expect("start succeeds");

        // Drop everything that holds a rewind_tx clone: the controller, the
        // server itself (its own sender), so the receiver inside the task
        // observes a fully-closed channel.
        drop(controller);
        drop(server);

        // Give the runtime a moment to deliver the channel-closed notification
        // to the task. Tokio's `mpsc::Receiver::recv` returns `None` once all
        // senders are dropped, and the `select!` arm with that pattern simply
        // never matches again — the other arms must keep working.
        tokio::time::sleep(Duration::from_millis(50)).await;

        assert!(
            !handle.task_handle.is_finished(),
            "drain task must survive a closed rewind channel"
        );

        handle.stop();
        handle.stopped().await;
    }
}
