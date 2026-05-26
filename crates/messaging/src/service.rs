use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use anyhow::Context;
use futures::StreamExt;
use katana_pool::api::TransactionPool;
use katana_pool::TxPool;
use katana_primitives::chain::ChainId;
use katana_primitives::transaction::{ExecutableTxWithHash, L1HandlerTx, TxHash};
use katana_provider::api::messaging::{
    MessagingCheckpoint, MessagingCheckpointProvider, MessagingL1ToL2IndexWriter,
};
use katana_provider::{MutableProvider, ProviderFactory, ProviderRW};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::stream::collector::ethereum::EthereumCollector;
use crate::stream::collector::starknet::StarknetCollector;
use crate::stream::collector::OrderedMessage;
use crate::stream::trigger::IntervalTrigger;
use crate::stream::MessageStream;
use crate::{MessagingOutcome, Messenger, SettlementChainConfig, LOG_TARGET};

/// Identifier used to namespace the persisted messaging checkpoint within the
/// shared `MessagingCheckpoints` table.
const CHECKPOINT_ID: &str = "messaging";

/// Default poll interval (in seconds) between gather ticks.
const DEFAULT_INTERVAL: u64 = 2;

/// The messaging server.
///
/// [`Self::start`] is non-consuming and mirrors `RpcServer::start`. It reads the
/// resume checkpoint, builds the messenger, and spawns the drain loop. A
/// settlement chain must be configured via [`settlement`](Self::settlement)
/// before calling [`start`](Self::start); otherwise it returns an error.
///
/// The server depends directly on the provider factory `P` for both reading the
/// resume checkpoint at boot and atomically persisting the L1->L2 index entry +
/// checkpoint after each successful pool insert.
///
/// Configure the server using the builder-style setters
/// ([`settlement`](Self::settlement), [`interval`](Self::interval),
/// [`from_block`](Self::from_block), [`confirmation_depth`](Self::confirmation_depth))
/// before calling [`start`](Self::start).
pub struct MessagingService<P, Pl = TxPool> {
    chain_id: ChainId,
    pool: Pl,
    provider: P,

    settlement: SettlementChainConfig,
    interval: u64,
    from_block: u64,
    confirmation_depth: u64,
}

impl<P, Pl> MessagingService<P, Pl> {
    /// Create a new messaging server with no settlement configured.
    ///
    /// A settlement chain must be set via [`settlement`](Self::settlement)
    /// before [`start`](Self::start) can be called.
    pub fn new(
        chain_id: ChainId,
        pool: Pl,
        provider: P,
        settlement: SettlementChainConfig,
    ) -> Self {
        Self {
            chain_id,
            pool,
            provider,
            settlement,
            interval: DEFAULT_INTERVAL,
            from_block: 0,
            confirmation_depth: 0,
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
    /// Start the messaging server.
    ///
    /// Reads the resume checkpoint, builds the messenger, and spawns the drain
    /// loop. Returns an error if no settlement chain has been configured via
    /// [`settlement`](Self::settlement).
    pub fn start(&self) -> Result<MessagingServiceHandle, anyhow::Error> {
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
        let pending = PendingMessages::default();
        let pending_for_task = pending.clone();
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

                                // Mark the whole batch as pending the moment it's gathered.
                                // Each entry is removed below once its pool insert succeeds;
                                // any left over after an error-break stays pending and is
                                // reconciled (overwritten by key) on the next gather.
                                pending_for_task.insert_batch(&messages);

                                for msg in messages {
                                    let hash = msg.tx.calculate_hash();
                                    info!(target: LOG_TARGET, tx_hash = %format!("{:#x}", hash), "L1Handler transaction added to the pool.");

                                    let pool_tx = ExecutableTxWithHash { hash, transaction: msg.tx.into() };
                                    let insert_result = pool.add_transaction(pool_tx).await;

                                    match insert_result {
                                        Ok(_) => {
                                            inserted += 1;

                                            // Accepted by the pool: no longer pending.
                                            pending_for_task.remove(msg.block, msg.tx_index);

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

                    _ = &mut shutdown => {
                        break;
                    }
                }
            }
        });

        info!(target: LOG_TARGET, "Messaging service started.");

        Ok(MessagingServiceHandle { shutdown_tx: Some(shutdown_tx), task_handle, pending })
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
    let cp = db_tx.messaging_checkpoint(CHECKPOINT_ID).context("read messaging checkpoint")?;
    db_tx.commit().context("commit checkpoint read tx")?;

    Ok(match cp {
        Some(c) => (c.block, c.tx_index + 1),
        None => (default_from_block, 0),
    })
}

impl<P, Pl> std::fmt::Debug for MessagingService<P, Pl> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessagingServer").finish_non_exhaustive()
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

/// A message that has been gathered from the settlement chain but not yet
/// accepted by the transaction pool.
#[derive(Debug, Clone)]
pub struct PendingMessage {
    /// The settlement block the message was emitted in.
    pub block: u64,
    /// The transaction index within `block`.
    pub tx_index: u64,
    /// The settlement-chain transaction hash that emitted the originating event/log.
    pub l1_tx_hash: [u8; 32],
    /// The hash of the L2 `L1Handler` transaction this message will become.
    pub l2_tx_hash: TxHash,
    /// The `L1Handler` transaction converted from the settlement-chain event.
    pub tx: L1HandlerTx,
}

/// Volatile, in-memory registry of messages that have been gathered from the
/// settlement chain but not yet accepted by the transaction pool.
///
/// A message is inserted the moment its batch is gathered ("picked up") and
/// removed the instant the pool accepts it. Entries are keyed by their
/// `(block, tx_index)` position, so a message re-gathered after a failed pool
/// insert overwrites its prior entry rather than duplicating it.
///
/// This state is intentionally volatile: it is created fresh on each
/// [`MessagingService::start`] and discarded on shutdown. It reflects only the
/// in-flight window of the currently running drain loop, never history.
#[derive(Debug, Clone, Default)]
pub struct PendingMessages {
    inner: Arc<Mutex<BTreeMap<(u64, u64), PendingMessage>>>,
}

impl PendingMessages {
    /// Mark every message in a freshly gathered batch as pending.
    fn insert_batch(&self, messages: &[OrderedMessage]) {
        let mut guard = self.inner.lock().expect("pending messages lock poisoned");
        for msg in messages {
            guard.insert(
                (msg.block, msg.tx_index),
                PendingMessage {
                    block: msg.block,
                    tx_index: msg.tx_index,
                    l1_tx_hash: msg.l1_tx_hash,
                    l2_tx_hash: msg.tx.calculate_hash(),
                    tx: msg.tx.clone(),
                },
            );
        }
    }

    /// Drop the entry for a message the pool has accepted.
    fn remove(&self, block: u64, tx_index: u64) {
        self.inner.lock().expect("pending messages lock poisoned").remove(&(block, tx_index));
    }

    /// Snapshot the currently pending messages, ordered by `(block, tx_index)`.
    pub fn snapshot(&self) -> Vec<PendingMessage> {
        self.inner.lock().expect("pending messages lock poisoned").values().cloned().collect()
    }

    /// The number of messages currently pending.
    pub fn len(&self) -> usize {
        self.inner.lock().expect("pending messages lock poisoned").len()
    }

    /// Whether there are no pending messages.
    pub fn is_empty(&self) -> bool {
        self.inner.lock().expect("pending messages lock poisoned").is_empty()
    }
}

/// Handle to a running messaging server, providing lifecycle control.
pub struct MessagingServiceHandle {
    shutdown_tx: Option<oneshot::Sender<()>>,
    task_handle: JoinHandle<()>,
    pending: PendingMessages,
}

impl std::fmt::Debug for MessagingServiceHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessagingHandle").finish_non_exhaustive()
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

    /// Snapshot the messages gathered from the settlement chain but not yet
    /// accepted by the transaction pool, ordered by `(block, tx_index)`.
    ///
    /// This is a point-in-time view of volatile in-memory state; an empty result
    /// means nothing is currently in flight, not that no messages were ever seen.
    pub fn pending_messages(&self) -> Vec<PendingMessage> {
        self.pending.snapshot()
    }
}

#[cfg(test)]
mod tests {
    use katana_primitives::Felt;
    use katana_provider::api::messaging::MessagingL1ToL2IndexProvider;
    use katana_provider::DbProviderFactory;

    use super::*;

    /// Builds a stub `OrderedMessage` whose tx internals don't matter — the buffer
    /// only keys on `(block, tx_index)`. `nonce` is varied so distinct positions
    /// produce distinct computed L2 hashes, letting tests tell entries apart.
    fn msg(block: u64, tx_index: u64) -> OrderedMessage {
        let tx = L1HandlerTx {
            calldata: vec![],
            chain_id: ChainId::default(),
            message_hash: Default::default(),
            paid_fee_on_l1: 0,
            nonce: Felt::from(block * 1000 + tx_index),
            entry_point_selector: Felt::ZERO,
            version: Felt::ZERO,
            contract_address: Default::default(),
        };
        OrderedMessage { block, tx_index, l1_tx_hash: [0u8; 32], tx }
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
        db_tx
            .set_messaging_checkpoint(
                CHECKPOINT_ID,
                &MessagingCheckpoint { block: 100, tx_index: 5 },
            )
            .unwrap();
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
        let cp =
            db_tx.messaging_checkpoint(CHECKPOINT_ID).unwrap().expect("checkpoint should exist");
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
        let cp = db_tx.messaging_checkpoint(CHECKPOINT_ID).unwrap().expect("checkpoint");
        db_tx.commit().unwrap();

        assert_eq!(cp.block, 10, "checkpoint should reflect the latest committed message");
        assert_eq!(cp.tx_index, 2);
    }

    /// A gathered batch becomes pending in `(block, tx_index)` order, and each entry
    /// disappears once its position is removed (the pool-accepted path).
    #[test]
    fn pending_messages_track_batch_then_clear_on_remove() {
        let pending = PendingMessages::default();
        assert!(pending.is_empty());

        // Intentionally out of order — the snapshot must come back sorted.
        pending.insert_batch(&[msg(7, 1), msg(5, 0), msg(7, 0)]);

        let snapshot = pending.snapshot();
        let positions: Vec<_> = snapshot.iter().map(|m| (m.block, m.tx_index)).collect();
        assert_eq!(
            positions,
            vec![(5, 0), (7, 0), (7, 1)],
            "snapshot should be ordered by (block, tx_index)"
        );
        assert_eq!(pending.len(), 3);

        // The pool accepts (5, 0): it leaves the pending set.
        pending.remove(5, 0);

        let positions: Vec<_> = pending.snapshot().iter().map(|m| (m.block, m.tx_index)).collect();
        assert_eq!(positions, vec![(7, 0), (7, 1)], "removed position should be gone");
    }

    /// Re-gathering the same message after a failed pool insert must not duplicate it:
    /// `(block, tx_index)` is the key, so a re-insert overwrites the prior entry.
    #[test]
    fn insert_batch_is_idempotent_by_position() {
        let pending = PendingMessages::default();

        pending.insert_batch(&[msg(3, 0), msg(3, 1)]);
        // Next tick re-gathers the unprocessed tail alongside fresh messages.
        pending.insert_batch(&[msg(3, 1), msg(4, 0)]);

        let positions: Vec<_> = pending.snapshot().iter().map(|m| (m.block, m.tx_index)).collect();
        assert_eq!(
            positions,
            vec![(3, 0), (3, 1), (4, 0)],
            "re-gathered (3, 1) should not duplicate"
        );
    }

    /// The computed L2 hash and the L1 origin hash are surfaced on each entry so a
    /// consumer can correlate a pending message with the pool/block once it lands.
    #[test]
    fn pending_message_exposes_l1_and_l2_hashes() {
        let pending = PendingMessages::default();
        let mut m = msg(1, 0);
        m.l1_tx_hash = [0xab; 32];
        let expected_l2 = m.tx.calculate_hash();

        pending.insert_batch(&[m]);

        let entry = pending.snapshot().pop().expect("one pending entry");
        assert_eq!(entry.l1_tx_hash, [0xab; 32]);
        assert_eq!(entry.l2_tx_hash, expected_l2);
    }
}
