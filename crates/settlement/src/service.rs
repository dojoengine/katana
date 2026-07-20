//! The settlement service: a single sequential settle loop.
//!
//! Settlement is inherently serial — Piltover rejects any `update_state` that
//! doesn't extend its current state — so the service runs one batch at a time
//! through the proving backend, with no internal pipelining. The loop is
//! agnostic to how a state transition is proven ([`ProvingBackend`]); the
//! chain side (the Piltover core contract on a Starknet chain) is concrete.

use std::sync::Arc;

use katana_primitives::block::BlockNumber;
use katana_primitives::settlement::{PendingBatchProof, ProofId};
use katana_primitives::transaction::TxHash;
use katana_provider::api::block::BlockNumberProvider;
use katana_provider::api::settlement::{
    SettlementCheckpointWriter, SettlementProofProvider, SettlementProofWriter,
};
use katana_provider::{MutableProvider, ProviderFactory};
use tokio::sync::{broadcast, oneshot};
use tokio::task::JoinHandle;
use tokio::time::{Duration, Instant};
use tracing::{error, info, warn};

use crate::backend::ProvingBackend;
use crate::error::SettlementError;
use crate::metrics::{SettlementMetrics, SettlementProofMetrics};
use crate::piltover::{PiltoverClient, PiltoverError};
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
    <P as ProviderFactory>::Provider: BlockNumberProvider + SettlementProofProvider,
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
    metrics: SettlementMetrics,
    proof_metrics: SettlementProofMetrics,
}

/// Outcome of a [`Worker::settle_batch`] attempt that did not fail terminally.
#[derive(Debug)]
enum SettleOutcome {
    /// The batch landed on the settlement chain.
    Settled { tx_hash: TxHash, proof: Option<ProofId> },
    /// Shutdown was requested while retrying the submission.
    ShuttingDown,
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

/// Reads the persisted generated-but-not-yet-settled proof reference. Best-effort: a read
/// failure only forfeits a potential recovery, so it degrades to `None` with a warning.
fn read_pending_batch_proof<P>(provider: &P) -> Option<PendingBatchProof>
where
    P: ProviderFactory,
    <P as ProviderFactory>::Provider: SettlementProofProvider,
{
    match provider.provider().pending_batch_proof() {
        Ok(pending) => pending,
        Err(error) => {
            warn!(%error, "Failed to read pending batch proof reference.");
            None
        }
    }
}

/// Records the network reference of a generated-but-not-yet-settled proof, replacing any
/// previous record — read back by [`read_pending_batch_proof`] after a restart. Best-effort:
/// a failed write only forfeits a potential recovery, never stalls settlement.
fn persist_pending_batch_proof<P>(
    provider: &P,
    first: BlockNumber,
    last: BlockNumber,
    proof: ProofId,
) where
    P: ProviderFactory,
    <P as ProviderFactory>::ProviderMut: SettlementProofWriter + MutableProvider,
{
    let db = provider.provider_mut();
    let result = db
        .set_pending_batch_proof(PendingBatchProof { first, last, proof })
        .and_then(|()| db.commit());
    if let Err(error) = result {
        warn!(%error, first, last, "Failed to persist pending batch proof reference.");
    }
}

/// Clears the pending proof reference once its batch settles. Best-effort: a stale record is
/// harmless — it can never match a future range (the cursor moved past it), so it is simply
/// overwritten by the next proving round.
fn clear_pending_batch_proof<P>(provider: &P)
where
    P: ProviderFactory,
    <P as ProviderFactory>::ProviderMut: SettlementProofWriter + MutableProvider,
{
    let db = provider.provider_mut();
    let result = db.clear_pending_batch_proof().and_then(|()| db.commit());
    if let Err(error) = result {
        warn!(%error, "Failed to clear pending batch proof reference.");
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
    <P as ProviderFactory>::Provider: BlockNumberProvider + SettlementProofProvider,
    <P as ProviderFactory>::ProviderMut:
        SettlementCheckpointWriter + SettlementProofWriter + MutableProvider,
{
    async fn run<N: Clone>(
        mut self,
        mut notify_rx: broadcast::Receiver<N>,
        // `oneshot::Receiver` is `Unpin`, so it can be polled through a plain `&mut` — both
        // here and inside `settle_batch`'s submission-retry loop, which borrows it to stay
        // shutdown-responsive mid-retry.
        mut shutdown_rx: oneshot::Receiver<()>,
    ) {
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
                    match self.settle_batch(first, last, &mut shutdown_rx).await {
                        Ok(SettleOutcome::ShuttingDown) => break,

                        Ok(SettleOutcome::Settled { tx_hash, proof }) => {
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

                        // A terminal attempt failure: proving failed, the chain rejected the
                        // payload in execution, or the on-chain cursor moved mid-attempt
                        // (`settle_batch` already updated it). Transient submission failures
                        // never surface here — they are retried inside `settle_batch` with
                        // the same proof.
                        Err(error) => {
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

    /// Settles the inclusive block range `[first, last]`: prove once, then retry the
    /// `update_state` submission until it lands or retrying stops making sense.
    ///
    /// The payload is obtained at most once per call — recovered from the proving network when
    /// a persisted [`PendingBatchProof`] reference from a previous run matches the range,
    /// otherwise proven fresh (persisting a new reference *first*, so a crash mid-attempt can
    /// recover the already-paid-for proof). It then lives as a local for exactly the duration
    /// of the attempt: transient submission failures (fees, nonce, transport) retry with the
    /// same payload, because on the SP1 prover network every proving round is paid work while
    /// the payload stays valid indefinitely for its fixed historical range — on-chain
    /// validation checks the proof against the range-derived commitment (state roots, block
    /// hashes, messages commitment), not any timestamp embedded at proving time.
    ///
    /// Terminal `Err` conditions end the attempt and hand control back to the run loop:
    ///
    /// - proving (or recovery followed by proving) failed;
    /// - the settlement chain *rejected the payload in execution* — the transaction reverted, e.g.
    ///   a TEE-registry trust-root rotation invalidated the proof between proving and submission.
    ///   The persisted reference is dropped so the next attempt proves fresh;
    /// - the on-chain cursor advanced mid-attempt (the transaction landed despite a reported error,
    ///   or another operator settled): the payload is spent or superseded, and the run loop
    ///   recomputes the next range from the updated cursor.
    ///
    /// [`PendingBatchProof`]: katana_primitives::settlement::PendingBatchProof
    async fn settle_batch(
        &mut self,
        first: BlockNumber,
        last: BlockNumber,
        shutdown_rx: &mut oneshot::Receiver<()>,
    ) -> Result<SettleOutcome, SettlementError> {
        let prev_block = if first == 0 { None } else { Some(first - 1) };

        // A persisted reference from a previous run lets the backend recover the
        // already-generated proof from the proving network instead of paying for a fresh
        // round. Best-effort: a reference for a different range is dead (the cursor moved
        // past it), and any recovery miss or failure just means proving fresh.
        let mut recovered = None;
        if let Some(pending) = read_pending_batch_proof(&self.provider) {
            if pending.first == first && pending.last == last {
                match self.backend.recover(prev_block, last, &pending.proof).await {
                    Ok(Some(payload)) => {
                        info!(
                            first,
                            last,
                            proof = ?pending.proof,
                            "Recovered proof from the proving network."
                        );
                        recovered = Some(payload);
                    }
                    // The backend cannot recover proofs (e.g. mock proving) — prove below.
                    Ok(None) => {}
                    Err(error) => {
                        warn!(
                            first,
                            last,
                            proof = ?pending.proof,
                            %error,
                            "Failed to recover proof; proving fresh."
                        );
                    }
                }
            }
        }

        let (update, proof) = match recovered {
            Some(payload) => payload,
            None => {
                let proof_start = Instant::now();
                let (update, proof) = match self.backend.prove(prev_block, last).await {
                    Ok(payload) => payload,
                    Err(error) => {
                        self.metrics.settlement_failures_total.increment(1);
                        return Err(error);
                    }
                };
                self.proof_metrics
                    .proof_generation_seconds
                    .record(proof_start.elapsed().as_secs_f64());

                // Record the network reference before attempting submission, so a crash
                // between proving and settling doesn't strand the proof.
                if let Some(proof) = &proof {
                    persist_pending_batch_proof(&self.provider, first, last, proof.clone());
                }

                (update, proof)
            }
        };

        // Submission retry loop: the payload never changes, only the attempt.
        let mut backoff = RETRY_BACKOFF_MIN;
        let mut submit_failures: u32 = 0;
        loop {
            let update_start = Instant::now();
            let error = match self.piltover.update_state(&update).await {
                Ok(tx_hash) => {
                    self.metrics.state_update_seconds.record(update_start.elapsed().as_secs_f64());
                    // The payload is spent — and its persisted reference with it.
                    clear_pending_batch_proof(&self.provider);
                    return Ok(SettleOutcome::Settled { tx_hash, proof });
                }
                Err(error) => error,
            };

            self.metrics.settlement_failures_total.increment(1);
            submit_failures += 1;

            // An execution revert means the payload itself was rejected on-chain — retrying
            // (or ever recovering) it cannot succeed.
            if matches!(error, PiltoverError::TransactionReverted(_)) {
                warn!(
                    first,
                    last,
                    "Settlement chain rejected the payload in execution; dropping it to prove \
                     fresh."
                );
                clear_pending_batch_proof(&self.provider);
                return Err(error.into());
            }

            error!(
                first,
                last,
                %error,
                submit_failures,
                retry_in = ?backoff,
                "Failed to submit state update; will retry with the same proof."
            );

            tokio::select! {
                _ = &mut *shutdown_rx => return Ok(SettleOutcome::ShuttingDown),
                _ = tokio::time::sleep(backoff) => {}
            }
            backoff = (backoff * 2).min(RETRY_BACKOFF_MAX);

            // The transaction may have landed even though we saw an error (e.g. a transient
            // RPC failure while watching the receipt). Re-reading the on-chain cursor makes
            // the retry idempotent: if it advanced, this payload is spent (or superseded)
            // and the run loop recomputes instead of double-submitting.
            match self.piltover.settled_block().await {
                Ok(cursor) if cursor != self.cursor => {
                    warn!(
                        ?cursor,
                        previous = ?self.cursor,
                        "On-chain settlement cursor advanced despite the error; continuing \
                         from it."
                    );
                    self.cursor = cursor;
                    if let Some(settled) = cursor {
                        persist_settled_block(&self.provider, settled);
                    }
                    return Err(error.into());
                }
                Ok(_) => {}
                Err(error) => {
                    error!(%error, "Failed to re-read on-chain settlement cursor.");
                }
            }
        }
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

    /// Exercises `settle_batch`'s prove-once semantics through its real submission-retry loop.
    ///
    /// The Piltover endpoint is a dummy that refuses connections, so every submission attempt
    /// fails with a transport error — the incident shape (the payload is fine, the chain is
    /// unreachable / the account can't pay). The tests run under a paused tokio clock, so the
    /// retry backoffs elapse instantly, and a virtual-time shutdown timer bounds the loop.
    mod proof_reuse {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        use async_trait::async_trait;
        use katana_primitives::block::BlockNumber;
        use katana_primitives::settlement::ProofId;
        use katana_provider::DbProviderFactory;
        use piltover::{PiltoverInput, TEEInput};
        use starknet::core::types::Felt;
        use tokio::sync::oneshot;
        use tokio::time::Duration;
        use url::Url;

        use crate::backend::ProvingBackend;
        use crate::error::SettlementError;
        use crate::metrics::{SettlementMetrics, SettlementProofMetrics};
        use crate::piltover::{PiltoverClient, PiltoverError};
        use crate::service::{
            clear_pending_batch_proof, read_pending_batch_proof, SettleOutcome, Worker,
        };

        /// How the mock backend answers `recover` calls.
        enum RecoverBehavior {
            /// Recovery succeeds with a payload (network still retains the proof).
            Payload,
            /// The backend cannot recover proofs at all (e.g. mock proving).
            Unsupported,
            /// Recovery was attempted and failed (e.g. the network expired the proof).
            Fail,
        }

        /// Counts `prove`/`recover` calls; `prove` fails when `fail` is set. The proof id
        /// encodes the prove-call count so tests can tell which round produced a payload.
        struct CountingBackend {
            calls: AtomicUsize,
            recover_calls: AtomicUsize,
            fail: bool,
            recover: RecoverBehavior,
        }

        impl CountingBackend {
            fn new(fail: bool) -> Self {
                Self {
                    calls: AtomicUsize::new(0),
                    recover_calls: AtomicUsize::new(0),
                    fail,
                    recover: RecoverBehavior::Unsupported,
                }
            }

            fn with_recover(recover: RecoverBehavior) -> Self {
                Self {
                    calls: AtomicUsize::new(0),
                    recover_calls: AtomicUsize::new(0),
                    fail: false,
                    recover,
                }
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
                Ok((test_input(block), Some(ProofId::new(vec![call as u8]))))
            }

            async fn recover(
                &self,
                _prev_block: Option<BlockNumber>,
                block: BlockNumber,
                proof: &ProofId,
            ) -> Result<Option<(PiltoverInput, Option<ProofId>)>, SettlementError> {
                self.recover_calls.fetch_add(1, Ordering::SeqCst);
                match self.recover {
                    RecoverBehavior::Payload => Ok(Some((test_input(block), Some(proof.clone())))),
                    RecoverBehavior::Unsupported => Ok(None),
                    RecoverBehavior::Fail => Err(SettlementError::Piltover(
                        PiltoverError::GetState("mock recovery failure".into()),
                    )),
                }
            }
        }

        fn test_input(block: BlockNumber) -> PiltoverInput {
            PiltoverInput::TeeInput(TEEInput {
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
            })
        }

        fn test_worker(
            backend: Arc<dyn ProvingBackend>,
            provider: DbProviderFactory,
        ) -> Worker<DbProviderFactory> {
            // The Piltover client's construction is pure field storage (no I/O); the dummy
            // endpoint makes every submission and cursor read fail with a transport error.
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
                provider,
                batch_size: 10,
                idle_flush_interval: Duration::from_secs(120),
                cursor: None,
                metrics: SettlementMetrics::default(),
                proof_metrics: SettlementProofMetrics::new_with_labels(&[("proof_type", "mock")]),
            }
        }

        /// Runs `settle_batch` against the unreachable Piltover endpoint, shutting the
        /// submission-retry loop down after `virtual_secs` of paused-clock time. Returns the
        /// outcome (either `ShuttingDown`, or `Err` for pre-submission failures).
        async fn settle_until_shutdown(
            worker: &mut Worker<DbProviderFactory>,
            first: BlockNumber,
            last: BlockNumber,
            virtual_secs: u64,
        ) -> Result<SettleOutcome, SettlementError> {
            let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(virtual_secs)).await;
                let _ = shutdown_tx.send(());
            });
            worker.settle_batch(first, last, &mut shutdown_rx).await
        }

        #[tokio::test(start_paused = true)]
        async fn proves_once_across_submission_retries() {
            let backend = Arc::new(CountingBackend::new(false));
            let mut worker = test_worker(backend.clone(), DbProviderFactory::new_in_memory());

            // 120 virtual seconds of transport failures spans several backoff cycles
            // (attempts at ~0s, 5s, 15s, 35s, 75s).
            let outcome = settle_until_shutdown(&mut worker, 5, 10, 120).await.unwrap();
            assert!(matches!(outcome, SettleOutcome::ShuttingDown));
            assert_eq!(backend.calls.load(Ordering::SeqCst), 1, "one prove for many submits");

            // The persisted network reference survives failed submissions — it is only spent
            // by a successful settle (or an execution revert).
            let pending = read_pending_batch_proof(&worker.provider).unwrap();
            assert_eq!((pending.first, pending.last), (5, 10));
        }

        #[tokio::test(start_paused = true)]
        async fn restart_recovers_proof_from_persisted_reference() {
            let provider = DbProviderFactory::new_in_memory();
            let backend = Arc::new(CountingBackend::with_recover(RecoverBehavior::Payload));

            // First "run": proving persists the pending network reference.
            let mut worker = test_worker(backend.clone(), provider.clone());
            settle_until_shutdown(&mut worker, 5, 10, 1).await.unwrap();
            assert_eq!(backend.calls.load(Ordering::SeqCst), 1);

            // "Restart": a fresh worker over the same DB recovers from the reference instead
            // of proving again.
            let mut worker = test_worker(backend.clone(), provider);
            let outcome = settle_until_shutdown(&mut worker, 5, 10, 1).await.unwrap();
            assert!(matches!(outcome, SettleOutcome::ShuttingDown));
            assert_eq!(backend.recover_calls.load(Ordering::SeqCst), 1);
            assert_eq!(backend.calls.load(Ordering::SeqCst), 1, "must not prove again");
        }

        #[tokio::test(start_paused = true)]
        async fn unrecoverable_reference_falls_back_to_fresh_proving() {
            for behavior in [RecoverBehavior::Unsupported, RecoverBehavior::Fail] {
                let provider = DbProviderFactory::new_in_memory();
                let backend = Arc::new(CountingBackend::with_recover(behavior));

                let mut worker = test_worker(backend.clone(), provider.clone());
                settle_until_shutdown(&mut worker, 5, 10, 1).await.unwrap();

                let mut worker = test_worker(backend.clone(), provider);
                settle_until_shutdown(&mut worker, 5, 10, 1).await.unwrap();
                assert_eq!(backend.recover_calls.load(Ordering::SeqCst), 1);
                assert_eq!(backend.calls.load(Ordering::SeqCst), 2, "fallback must prove");
            }
        }

        #[tokio::test(start_paused = true)]
        async fn stale_reference_for_other_range_skips_recovery() {
            let provider = DbProviderFactory::new_in_memory();
            let backend = Arc::new(CountingBackend::with_recover(RecoverBehavior::Payload));

            let mut worker = test_worker(backend.clone(), provider.clone());
            settle_until_shutdown(&mut worker, 5, 10, 1).await.unwrap();

            // The cursor moved while the node was down — the persisted reference no longer
            // matches the range to settle and must not even be attempted.
            let mut worker = test_worker(backend.clone(), provider);
            settle_until_shutdown(&mut worker, 11, 12, 1).await.unwrap();
            assert_eq!(backend.recover_calls.load(Ordering::SeqCst), 0);
            assert_eq!(backend.calls.load(Ordering::SeqCst), 2);
        }

        #[tokio::test(start_paused = true)]
        async fn prove_failure_returns_before_submission() {
            let backend = Arc::new(CountingBackend::new(true));
            let mut worker = test_worker(backend.clone(), DbProviderFactory::new_in_memory());

            settle_until_shutdown(&mut worker, 5, 10, 1).await.unwrap_err();
            assert!(read_pending_batch_proof(&worker.provider).is_none());
        }

        #[tokio::test(start_paused = true)]
        async fn cleared_reference_proves_fresh() {
            let provider = DbProviderFactory::new_in_memory();
            let backend = Arc::new(CountingBackend::with_recover(RecoverBehavior::Payload));

            let mut worker = test_worker(backend.clone(), provider.clone());
            settle_until_shutdown(&mut worker, 5, 10, 1).await.unwrap();
            assert!(read_pending_batch_proof(&provider).is_some());

            // What a successful settle (or an execution revert) does.
            clear_pending_batch_proof(&provider);
            assert!(read_pending_batch_proof(&provider).is_none());

            let mut worker = test_worker(backend.clone(), provider);
            settle_until_shutdown(&mut worker, 5, 10, 1).await.unwrap();
            assert_eq!(backend.recover_calls.load(Ordering::SeqCst), 0);
            assert_eq!(backend.calls.load(Ordering::SeqCst), 2);
        }
    }
}
