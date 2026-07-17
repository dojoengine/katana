//! The settlement service: a single sequential settle loop.
//!
//! Settlement is inherently serial — Piltover rejects any `update_state` that
//! doesn't extend its current state — so the service runs one batch at a time
//! through the proving backend, with no internal pipelining. The loop is
//! agnostic to how a state transition is proven ([`ProvingBackend`]); the
//! chain side (the Piltover core contract on a Starknet chain) is concrete.

use std::sync::Arc;

use katana_primitives::block::BlockNumber;
use katana_primitives::settlement::ProofId;
use katana_primitives::transaction::TxHash;
use katana_provider::api::block::BlockNumberProvider;
use katana_provider::api::settlement::{SettlementCheckpointWriter, SettlementProofWriter};
use katana_provider::{MutableProvider, ProviderFactory};
use piltover::PiltoverInput;
use tokio::sync::{broadcast, oneshot};
use tokio::task::JoinHandle;
use tokio::time::{Duration, Instant};
use tracing::{error, info, warn};

use crate::backend::ProvingBackend;
use crate::error::SettlementError;
use crate::metrics::{SettlementMetrics, SettlementProofMetrics};
use crate::piltover::PiltoverClient;
use crate::SettlementConfig;

/// Initial retry delay after a failed settlement attempt.
const RETRY_BACKOFF_MIN: Duration = Duration::from_secs(5);
/// Retry delay cap.
const RETRY_BACKOFF_MAX: Duration = Duration::from_secs(60);

/// The embedded settlement service.
///
/// [`Self::start`] is non-consuming and mirrors `MessagingService::start`: it
/// connects to the settlement chain, reads the on-chain cursor, and spawns the
/// settle loop. The broadcast channel is used purely as a new-block wake-up —
/// its payload is ignored, so any clonable type works (`N` is the node's mined
/// block notification type).
pub struct SettlementService<P, N> {
    provider: P,
    backend: Arc<dyn ProvingBackend>,
    block_notify: broadcast::Sender<N>,
    config: SettlementConfig,
}

impl<P, N> SettlementService<P, N> {
    pub fn new(
        provider: P,
        backend: Arc<dyn ProvingBackend>,
        block_notify: broadcast::Sender<N>,
        config: SettlementConfig,
    ) -> Self {
        Self { provider, backend, block_notify, config }
    }
}

impl<P, N> SettlementService<P, N>
where
    P: ProviderFactory + Clone + Send + Sync + 'static,
    <P as ProviderFactory>::Provider: BlockNumberProvider,
    <P as ProviderFactory>::ProviderMut:
        SettlementCheckpointWriter + SettlementProofWriter + MutableProvider,
    N: Clone + Send + 'static,
{
    /// Start the settlement service.
    ///
    /// Connects to the Piltover core contract, reads the settled-block cursor, and spawns the
    /// settle loop.
    pub async fn start(&self) -> Result<SettlementServiceHandle, SettlementError> {
        // validate core contract is configured correctly

        let piltover = PiltoverClient::new(
            self.config.rpc_url.clone(),
            self.config.chain_id,
            self.config.core_contract,
            self.config.account_address,
            self.config.account_private_key,
        );

        let cursor = piltover.settled_block().await?;

        // Seed the durable checkpoint with the authoritative on-chain cursor: if the chain is
        // already caught up, no settle happens and this is the only write the index would get.
        if let Some(settled) = cursor {
            persist_settled_block(&self.provider, settled);
        }

        let worker = Worker {
            cursor,
            piltover,
            prepared: None,
            backend: self.backend.clone(),
            provider: self.provider.clone(),
            batch_size: self.config.batch_size.max(1) as u64,
            idle_flush_interval: self.config.idle_flush_interval,
            metrics: SettlementMetrics::default(),
            proof_metrics: SettlementProofMetrics::new_with_labels(&[(
                "proof_type",
                self.backend.proof_type(),
            )]),
        };

        let notify_rx = self.block_notify.subscribe();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let task_handle = tokio::spawn(worker.run(notify_rx, shutdown_rx));

        info!(
            backend = self.backend.name(),
            settled_block = ?cursor,
            settlement_chain = %self.config.chain_id,
            core_contract = %self.config.core_contract,
            batch_size = %self.config.batch_size,
            "Settlement service started."
        );

        Ok(SettlementServiceHandle { shutdown_tx: Some(shutdown_tx), task_handle })
    }
}

impl<P, N> std::fmt::Debug for SettlementService<P, N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettlementService")
            .field("backend", &self.backend.name())
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

/// Handle to a running settlement service.
#[derive(Debug)]
pub struct SettlementServiceHandle {
    shutdown_tx: Option<oneshot::Sender<()>>,
    task_handle: JoinHandle<()>,
}

impl SettlementServiceHandle {
    /// Signal the service to shut down.
    pub fn stop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }

    /// Wait for the service task to fully terminate.
    pub async fn stopped(self) {
        let _ = self.task_handle.await;
    }
}

struct Worker<P> {
    provider: P,
    backend: Arc<dyn ProvingBackend>,
    piltover: PiltoverClient,
    batch_size: u64,
    idle_flush_interval: tokio::time::Duration,
    /// Last settled block, from Piltover's `get_state()`. `None` = nothing settled yet.
    cursor: Option<BlockNumber>,
    /// Proving result awaiting on-chain acceptance — see [`PreparedBatch`].
    prepared: Option<PreparedBatch>,
    metrics: SettlementMetrics,
    proof_metrics: SettlementProofMetrics,
}

/// A proving result for a settle range that has not yet been accepted on-chain.
///
/// Kept across retries so a failed `update_state` submission (fees, nonce, a transient RPC
/// error) does not trigger a fresh — and, on the SP1 prover network, paid — proving round for
/// the identical range. The payload stays valid indefinitely for a fixed historical range:
/// on-chain validation checks the proof against the range-derived commitment (state roots,
/// block hashes, messages commitment), not any timestamp embedded at proving time.
///
/// In-memory only: a node restart clears it, which doubles as the escape hatch in the rare
/// case a cached proof is invalidated externally (e.g. an on-chain TEE-registry trust-root
/// rotation between proving and submission).
struct PreparedBatch {
    first: BlockNumber,
    last: BlockNumber,
    update: PiltoverInput,
    proof: Option<ProofId>,
}

/// Persists the settled-block cursor to the durable [`tables::SettlementCheckpoints`] index, read
/// back by the `katana_settlementStatus` RPC. Best-effort: the on-chain Piltover cursor is the
/// authoritative source of progress, so a failed write is logged but never stalls settlement.
///
/// [`tables::SettlementCheckpoints`]: katana_db::tables::SettlementCheckpoints
fn persist_settled_block<P>(provider: &P, block: BlockNumber)
where
    P: ProviderFactory,
    <P as ProviderFactory>::ProviderMut: SettlementCheckpointWriter,
{
    let db = provider.provider_mut();
    let result = db.set_settled_block(block).and_then(|()| db.commit());
    if let Err(error) = result {
        warn!(%error, block, "Failed to persist settled-block checkpoint.");
    }
}

/// Persists the block -> proof mapping for the settled range `[first, last]` to the durable
/// [`tables::SettlementProofs`] index, read back by the `katana_getBlockProof` RPC. Every block in
/// the batch was settled by the same proof, so they all map to `proof`. Best-effort, matching
/// [`persist_settled_block`]: a failed write is logged but never stalls settlement.
///
/// [`tables::SettlementProofs`]: katana_db::tables::SettlementProofs
fn persist_block_proofs<P>(provider: &P, first: BlockNumber, last: BlockNumber, proof: ProofId)
where
    P: ProviderFactory,
    <P as ProviderFactory>::ProviderMut: SettlementProofWriter + MutableProvider,
{
    let db = provider.provider_mut();
    let result = (first..=last)
        .try_for_each(|block| db.set_block_proof(block, proof.clone()))
        .and_then(|()| db.commit());
    if let Err(error) = result {
        warn!(%error, first, last, "Failed to persist block -> proof mapping.");
    }
}

/// What the settle loop should do next, given the current durable state.
#[derive(Debug, PartialEq, Eq)]
enum Action {
    /// Settle this inclusive block range now.
    Settle { first: BlockNumber, last: BlockNumber },
    /// Blocks are pending but the batch is partial — wait for more blocks or the idle deadline.
    WaitForBatch,
    /// Fully caught up — wait for a new block.
    Idle,
}

/// Pure batching decision: drives both the run loop and the unit tests.
///
/// `cursor` is the last settled block (`None` = genesis not settled), `head` the local chain tip.
fn next_action(
    cursor: Option<BlockNumber>,
    head: BlockNumber,
    batch_size: u64,
    idle_elapsed: bool,
) -> Action {
    let next = cursor.map(|c| c + 1).unwrap_or(0);

    if head < next {
        return Action::Idle;
    }

    let pending = head - next + 1;
    if pending >= batch_size || idle_elapsed {
        Action::Settle { first: next, last: head.min(next + batch_size - 1) }
    } else {
        Action::WaitForBatch
    }
}

impl<P> Worker<P>
where
    P: ProviderFactory,
    <P as ProviderFactory>::Provider: BlockNumberProvider,
    <P as ProviderFactory>::ProviderMut:
        SettlementCheckpointWriter + SettlementProofWriter + MutableProvider,
{
    async fn run<N: Clone>(
        mut self,
        mut notify_rx: broadcast::Receiver<N>,
        shutdown_rx: oneshot::Receiver<()>,
    ) {
        tokio::pin!(shutdown_rx);

        let mut idle_deadline = Instant::now() + self.idle_flush_interval;
        let mut backoff = RETRY_BACKOFF_MIN;
        let mut consecutive_failures: u32 = 0;

        loop {
            let head = match self.local_head() {
                Ok(head) => head,
                Err(error) => {
                    error!(%error, "Failed to read local chain head.");
                    tokio::time::sleep(RETRY_BACKOFF_MIN).await;
                    continue;
                }
            };

            let idle_elapsed = Instant::now() >= idle_deadline;

            match next_action(self.cursor, head, self.batch_size, idle_elapsed) {
                Action::Settle { first, last } => {
                    let batch_start = Instant::now();
                    match self.settle_batch(first, last).await {
                        Ok((tx_hash, proof)) => {
                            let blocks = last - first + 1;
                            self.proof_metrics
                                .settle_batch_seconds
                                .record(batch_start.elapsed().as_secs_f64());
                            self.metrics.blocks_per_batch.record(blocks as f64);
                            self.metrics.batches_settled_total.increment(1);
                            self.metrics.blocks_settled_total.increment(blocks);
                            self.metrics.settled_block.set(last as f64);

                            info!(
                                first,
                                last,
                                tx_hash = %format!("{tx_hash:#x}"),
                                proof = ?proof,
                                "Settled block range."
                            );
                            self.cursor = Some(last);
                            persist_settled_block(&self.provider, last);
                            // Record which proof settled each block in the batch. Backends without
                            // a proof reference (e.g. mock proving)
                            // report `None`.
                            if let Some(proof) = proof {
                                persist_block_proofs(&self.provider, first, last, proof);
                            }
                            idle_deadline = Instant::now() + self.idle_flush_interval;
                            backoff = RETRY_BACKOFF_MIN;
                            consecutive_failures = 0;
                            // Loop again immediately: drain any remaining backlog.
                        }

                        Err(error) => {
                            self.metrics.settlement_failures_total.increment(1);
                            consecutive_failures += 1;
                            error!(
                                first,
                                last,
                                %error,
                                consecutive_failures,
                                retry_in = ?backoff,
                                "Failed to settle block range; will retry."
                            );

                            tokio::select! {
                                _ = &mut shutdown_rx => break,
                                _ = tokio::time::sleep(backoff) => {}
                            }
                            backoff = (backoff * 2).min(RETRY_BACKOFF_MAX);

                            // The transaction may have landed even though we saw an error (e.g.
                            // a transient RPC failure while watching the receipt). Re-reading
                            // the on-chain cursor makes the retry idempotent: if it advanced,
                            // the loop moves on instead of double-submitting.
                            match self.piltover.settled_block().await {
                                Ok(cursor) => {
                                    if cursor != self.cursor {
                                        warn!(
                                            ?cursor,
                                            previous = ?self.cursor,
                                            "On-chain settlement cursor advanced despite the \
                                             error; continuing from it."
                                        );
                                        self.cursor = cursor;
                                        if let Some(settled) = cursor {
                                            persist_settled_block(&self.provider, settled);
                                        }
                                    }
                                }
                                Err(error) => {
                                    error!(
                                        %error,
                                        "Failed to re-read on-chain settlement cursor."
                                    );
                                }
                            }
                        }
                    }
                }

                Action::WaitForBatch => {
                    tokio::select! {
                        _ = &mut shutdown_rx => break,
                        _ = tokio::time::sleep_until(idle_deadline) => {}
                        r = notify_rx.recv() => match r {
                            // New block mined — re-evaluate. The payload is irrelevant; the
                            // provider is re-read on the next iteration.
                            Ok(_) => {}

                            // Missed notifications are harmless: the provider is the source
                            // of truth and is re-read every iteration.
                            Err(broadcast::error::RecvError::Lagged(_)) => {}

                            // Sender dropped — node is shutting down; wait for the signal.
                            Err(broadcast::error::RecvError::Closed) => {
                                let _ = (&mut shutdown_rx).await;
                                break;
                            }
                        },
                    }
                }

                Action::Idle => {
                    tokio::select! {
                        _ = &mut shutdown_rx => break,
                        r = notify_rx.recv() => match r {
                            Ok(_) => {
                                // First block of a fresh batch window: arm the idle flush timer.
                                idle_deadline = Instant::now() + self.idle_flush_interval;
                            }
                            Err(broadcast::error::RecvError::Lagged(_)) => {}
                            Err(broadcast::error::RecvError::Closed) => {
                                let _ = (&mut shutdown_rx).await;
                                break;
                            }
                        },
                    }
                }
            }
        }

        info!("Settlement service stopped.");
    }

    /// Retrieve the latest block on the local chain.
    ///
    /// Errors if the chain has no blocks at all — which should not happen in normal operation,
    /// since the node commits the genesis block at startup. The run loop treats that error like
    /// any other transient read failure (log, back off, retry).
    fn local_head(&self) -> Result<BlockNumber, SettlementError> {
        self.provider.provider().latest_number().map_err(Into::into)
    }

    /// Prove and settle the inclusive block range `[first, last]`.
    ///
    /// Proving is skipped when a [`PreparedBatch`] for exactly this range is already held from
    /// a previous attempt whose submission failed — only a successful `update_state` clears it,
    /// so retries of the same range never re-prove.
    ///
    /// Returns the settlement-chain transaction hash and a reference to the proof that settled the
    /// batch (`None` for mock / off-network proving).
    async fn settle_batch(
        &mut self,
        first: BlockNumber,
        last: BlockNumber,
    ) -> Result<(TxHash, Option<ProofId>), SettlementError> {
        self.prepare_batch(first, last).await?;
        let prepared = self.prepared.as_ref().expect("prepare_batch always sets it");

        let update_start = Instant::now();
        let tx_hash = self.piltover.update_state(&prepared.update).await?;
        self.metrics.state_update_seconds.record(update_start.elapsed().as_secs_f64());

        // Only now is the payload spent — a submission failure above returns early and keeps
        // `self.prepared` for the next attempt.
        let proof = self.prepared.take().and_then(|p| p.proof);

        Ok((tx_hash, proof))
    }

    /// Ensures `self.prepared` holds the update payload + proof for `[first, last]`, proving
    /// only if no [`PreparedBatch`] for exactly this range is already held.
    ///
    /// A held batch for a *different* range is dead — the settle cursor moved, so that payload
    /// can never be submitted — and is dropped before proving the new range.
    async fn prepare_batch(
        &mut self,
        first: BlockNumber,
        last: BlockNumber,
    ) -> Result<(), SettlementError> {
        match &self.prepared {
            Some(batch) if batch.first == first && batch.last == last => {
                info!(first, last, "Reusing prepared proof for unchanged range.");
            }
            _ => {
                self.prepared = None;
                let prev_block = if first == 0 { None } else { Some(first - 1) };

                let proof_start = Instant::now();
                let (update, proof) = self.backend.prove(prev_block, last).await?;
                self.proof_metrics
                    .proof_generation_seconds
                    .record(proof_start.elapsed().as_secs_f64());

                self.prepared = Some(PreparedBatch { first, last, update, proof });
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{next_action, persist_block_proofs, Action};

    #[test]
    fn persist_block_proofs_maps_every_block_in_range() {
        use katana_primitives::settlement::ProofId;
        use katana_provider::api::settlement::SettlementProofProvider;
        use katana_provider::{DbProviderFactory, ProviderFactory};

        let factory = DbProviderFactory::new_in_memory();
        let proof = ProofId::new(vec![0x11; 32]);

        // A batch settling blocks 5..=8 maps every block to the same proof.
        persist_block_proofs(&factory, 5, 8, proof.clone());

        for block in 5..=8 {
            assert_eq!(factory.provider().block_proof(block).unwrap(), Some(proof.clone()));
        }
        // Blocks outside the settled range stay unmapped.
        assert_eq!(factory.provider().block_proof(4).unwrap(), None);
        assert_eq!(factory.provider().block_proof(9).unwrap(), None);
    }

    #[test]
    fn nothing_settled() {
        // Only the genesis block present, batch of 1 → settle block 0 immediately.
        assert_eq!(next_action(None, 0, 1, false), Action::Settle { first: 0, last: 0 });
        // Only the genesis block, larger batch → wait for more blocks (or the idle flush).
        assert_eq!(next_action(None, 0, 10, false), Action::WaitForBatch);
        // A few blocks present, batch not yet full → wait unless idle.
        assert_eq!(next_action(None, 2, 10, false), Action::WaitForBatch);
        assert_eq!(next_action(None, 2, 10, true), Action::Settle { first: 0, last: 2 });
    }

    #[test]
    fn backlog_drains_in_batches() {
        // 25 unsettled blocks, batch of 10 → settle the first 10.
        assert_eq!(next_action(Some(4), 29, 10, false), Action::Settle { first: 5, last: 14 });
        // After settling, the next call picks up the following range.
        assert_eq!(next_action(Some(14), 29, 10, false), Action::Settle { first: 15, last: 24 });
        // The remainder is a partial batch.
        assert_eq!(next_action(Some(24), 29, 10, false), Action::WaitForBatch);
        assert_eq!(next_action(Some(24), 29, 10, true), Action::Settle { first: 25, last: 29 });
    }

    #[test]
    fn caught_up_is_idle() {
        assert_eq!(next_action(Some(7), 7, 10, false), Action::Idle);
        assert_eq!(next_action(Some(7), 7, 10, true), Action::Idle);
        // Cursor ahead of head (e.g. fresh db against an old piltover) — nothing to do.
        assert_eq!(next_action(Some(9), 7, 10, true), Action::Idle);
    }

    #[test]
    fn idle_elapsed_flushes_partial_batch() {
        assert_eq!(next_action(Some(2), 4, 10, true), Action::Settle { first: 3, last: 4 });
    }

    mod proof_reuse {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        use async_trait::async_trait;
        use katana_primitives::block::BlockNumber;
        use katana_primitives::settlement::ProofId;
        use katana_provider::DbProviderFactory;
        use piltover::{PiltoverInput, TEEInput};
        use starknet::core::types::Felt;
        use url::Url;

        use crate::backend::ProvingBackend;
        use crate::error::SettlementError;
        use crate::metrics::{SettlementMetrics, SettlementProofMetrics};
        use crate::piltover::{PiltoverClient, PiltoverError};
        use crate::service::Worker;

        /// Counts `prove` calls; fails when `fail` is set. The proof id encodes the call count
        /// so tests can tell which proving round produced the held payload.
        struct CountingBackend {
            calls: AtomicUsize,
            fail: bool,
        }

        impl CountingBackend {
            fn new(fail: bool) -> Self {
                Self { calls: AtomicUsize::new(0), fail }
            }
        }

        #[async_trait]
        impl ProvingBackend for CountingBackend {
            fn name(&self) -> &'static str {
                "counting"
            }

            fn proof_type(&self) -> &'static str {
                "mock"
            }

            async fn prove(
                &self,
                _prev_block: Option<BlockNumber>,
                block: BlockNumber,
            ) -> Result<(PiltoverInput, Option<ProofId>), SettlementError> {
                let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
                if self.fail {
                    return Err(SettlementError::Piltover(PiltoverError::GetState(
                        "mock prove failure".into(),
                    )));
                }

                let input = PiltoverInput::TeeInput(TEEInput {
                    sp1_proof: vec![],
                    prev_state_root: Felt::ZERO,
                    state_root: Felt::ZERO,
                    prev_block_hash: Felt::ZERO,
                    block_hash: Felt::ZERO,
                    prev_block_number: Felt::ZERO,
                    block_number: Felt::from(block),
                    messages_commitment: Felt::ZERO,
                    messages_to_starknet: vec![],
                    messages_to_appchain: vec![],
                    l1_to_l2_msg_hashes: vec![],
                    katana_tee_config_hash: Felt::ZERO,
                });
                Ok((input, Some(ProofId::new(vec![call as u8]))))
            }
        }

        fn test_worker(backend: Arc<dyn ProvingBackend>) -> Worker<DbProviderFactory> {
            // The Piltover client is never used by `prepare_batch`; construction is pure field
            // storage (no I/O), so a dummy endpoint is fine.
            let piltover = PiltoverClient::new(
                Url::parse("http://127.0.0.1:1").unwrap(),
                Default::default(),
                Default::default(),
                Default::default(),
                Felt::ONE,
            );

            Worker {
                piltover,
                backend: backend.clone(),
                provider: DbProviderFactory::new_in_memory(),
                batch_size: 10,
                idle_flush_interval: tokio::time::Duration::from_secs(120),
                cursor: None,
                prepared: None,
                metrics: SettlementMetrics::default(),
                proof_metrics: SettlementProofMetrics::new_with_labels(&[("proof_type", "mock")]),
            }
        }

        fn held_proof(worker: &Worker<DbProviderFactory>) -> Option<&ProofId> {
            worker.prepared.as_ref().and_then(|p| p.proof.as_ref())
        }

        #[tokio::test]
        async fn retry_of_same_range_does_not_reprove() {
            let backend = Arc::new(CountingBackend::new(false));
            let mut worker = test_worker(backend.clone());

            worker.prepare_batch(5, 10).await.unwrap();
            assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
            let first_proof = held_proof(&worker).cloned();

            // A retry of the identical range (e.g. after a failed submission) reuses the held
            // payload instead of proving again.
            worker.prepare_batch(5, 10).await.unwrap();
            assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
            assert_eq!(held_proof(&worker).cloned(), first_proof);
        }

        #[tokio::test]
        async fn range_change_invalidates_held_batch() {
            let backend = Arc::new(CountingBackend::new(false));
            let mut worker = test_worker(backend.clone());

            worker.prepare_batch(5, 10).await.unwrap();
            // The cursor moved (e.g. the tx landed despite a reported error) — the new range
            // must be proven fresh.
            worker.prepare_batch(11, 12).await.unwrap();

            assert_eq!(backend.calls.load(Ordering::SeqCst), 2);
            let held = worker.prepared.as_ref().unwrap();
            assert_eq!((held.first, held.last), (11, 12));
        }

        #[tokio::test]
        async fn cleared_batch_is_reproven() {
            let backend = Arc::new(CountingBackend::new(false));
            let mut worker = test_worker(backend.clone());

            worker.prepare_batch(5, 10).await.unwrap();
            // `settle_batch` takes the payload on successful submission; the next range (even
            // an identical one, which cannot happen in practice) proves fresh.
            worker.prepared = None;
            worker.prepare_batch(5, 10).await.unwrap();

            assert_eq!(backend.calls.load(Ordering::SeqCst), 2);
        }

        #[tokio::test]
        async fn failed_submission_keeps_proof_for_retry() {
            let backend = Arc::new(CountingBackend::new(false));
            let mut worker = test_worker(backend.clone());

            // The dummy Piltover endpoint refuses connections, so submission fails after
            // proving succeeds — the production incident shape (e.g. account short on fees).
            worker.settle_batch(5, 10).await.unwrap_err();
            assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
            let held = worker.prepared.as_ref().expect("payload kept for retry");
            assert_eq!((held.first, held.last), (5, 10));

            // The retry fails at submission again but must not prove again.
            worker.settle_batch(5, 10).await.unwrap_err();
            assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
            assert!(worker.prepared.is_some());
        }

        #[tokio::test]
        async fn prove_failure_holds_nothing() {
            let backend = Arc::new(CountingBackend::new(true));
            let mut worker = test_worker(backend.clone());

            // Seed a stale entry for a different range: it must be dropped before the failing
            // prove, not resurrected after it.
            worker.prepared = Some(super::super::PreparedBatch {
                first: 1,
                last: 2,
                update: PiltoverInput::LayoutBridgeOutputNoDa(vec![]),
                proof: None,
            });

            worker.prepare_batch(5, 10).await.unwrap_err();
            assert!(worker.prepared.is_none());
        }
    }
}
