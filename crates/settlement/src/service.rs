//! The settlement service: a single sequential settle loop.
//!
//! Settlement is inherently serial — the core contract rejects any state
//! update that doesn't extend its current state — so the service runs one
//! batch at a time through the proving pipeline, with no internal pipelining.
//! The loop is agnostic to both axes of a settlement setup: how a state
//! transition is proven ([`ProvingBackend`]) and which chain it is submitted
//! to ([`SettlementChain`]). The two are paired — with their payload types
//! statically checked to match — in a [`SettlementPipeline`].

use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use katana_primitives::block::BlockNumber;
use katana_provider::api::block::BlockNumberProvider;
use katana_provider::{ProviderError, ProviderFactory};
use tokio::sync::{broadcast, oneshot};
use tokio::task::JoinHandle;
use tokio::time::{Duration, Instant};
use tracing::{error, info, warn};

use crate::backend::ProvingBackend;
use crate::chain::SettlementChain;
use crate::{SettlementConfig, LOG_TARGET};

/// Initial retry delay after a failed settlement attempt.
const RETRY_BACKOFF_MIN: Duration = Duration::from_secs(5);
/// Retry delay cap.
const RETRY_BACKOFF_MAX: Duration = Duration::from_secs(60);

/// A proving backend paired with the settlement chain it settles to.
///
/// The pairing is where the two axes meet: the backend's
/// [`Output`](ProvingBackend::Output) must be the payload type the chain's
/// core contract accepts ([`SettlementChain::StateUpdate`]) — checked
/// statically by this struct's bounds. The service consumes the pair through
/// the type-erased [`Settle`] trait so it stays agnostic to both.
pub struct SettlementPipeline<B, C> {
    backend: B,
    chain: C,
}

impl<B, C> SettlementPipeline<B, C>
where
    B: ProvingBackend,
    C: SettlementChain<StateUpdate = B::Output>,
{
    pub fn new(backend: B, chain: C) -> Self {
        Self { backend, chain }
    }
}

/// Type-erased view of a [`SettlementPipeline`], as consumed by the settle loop.
#[async_trait]
trait Settle: Send + Sync {
    fn backend_name(&self) -> &'static str;
    fn chain_name(&self) -> &'static str;

    /// Reads the settled-block cursor from the chain's core contract.
    async fn settled_block(&self) -> Result<Option<BlockNumber>>;

    /// Proves the state transition `(prev_block, block]` and submits it to
    /// the chain. Returns a displayable transaction id.
    async fn settle(&self, prev_block: Option<BlockNumber>, block: BlockNumber) -> Result<String>;
}

#[async_trait]
impl<B, C> Settle for SettlementPipeline<B, C>
where
    B: ProvingBackend,
    C: SettlementChain<StateUpdate = B::Output>,
{
    fn backend_name(&self) -> &'static str {
        self.backend.name()
    }

    fn chain_name(&self) -> &'static str {
        self.chain.name()
    }

    async fn settled_block(&self) -> Result<Option<BlockNumber>> {
        self.chain.settled_block().await
    }

    async fn settle(&self, prev_block: Option<BlockNumber>, block: BlockNumber) -> Result<String> {
        let update = self.backend.prove(prev_block, block).await?;
        self.chain.update_state(update).await
    }
}

/// The embedded settlement service.
///
/// [`Self::start`] is non-consuming and mirrors `MessagingService::start`: it
/// reads the on-chain cursor and spawns the settle loop. The broadcast channel
/// is used purely as a new-block wake-up — its payload is ignored, so any
/// clonable type works (`N` is the node's mined block notification type).
pub struct SettlementService<P, N> {
    provider: P,
    pipeline: Arc<dyn Settle>,
    block_notify: broadcast::Sender<N>,
    config: SettlementConfig,
}

impl<P, N> SettlementService<P, N> {
    pub fn new<B, C>(
        provider: P,
        pipeline: SettlementPipeline<B, C>,
        block_notify: broadcast::Sender<N>,
        config: SettlementConfig,
    ) -> Self
    where
        B: ProvingBackend + 'static,
        C: SettlementChain<StateUpdate = B::Output> + 'static,
    {
        Self { provider, pipeline: Arc::new(pipeline), block_notify, config }
    }
}

impl<P, N> SettlementService<P, N>
where
    P: ProviderFactory + Clone + Send + Sync + 'static,
    <P as ProviderFactory>::Provider: BlockNumberProvider,
    N: Clone + Send + 'static,
{
    /// Start the settlement service.
    ///
    /// Reads the settled-block cursor from the settlement chain's core contract and spawns the
    /// settle loop.
    pub async fn start(&self) -> Result<SettlementServiceHandle> {
        let cursor =
            self.pipeline.settled_block().await.context("read on-chain settlement cursor")?;

        let worker = Worker {
            provider: self.provider.clone(),
            pipeline: self.pipeline.clone(),
            batch_size: self.config.batch_size.max(1) as u64,
            idle_flush_interval: self.config.idle_flush_interval,
            cursor,
        };

        let notify_rx = self.block_notify.subscribe();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let task_handle = tokio::spawn(worker.run(notify_rx, shutdown_rx));

        info!(
            target: LOG_TARGET,
            backend = self.pipeline.backend_name(),
            chain = self.pipeline.chain_name(),
            settled_block = ?cursor,
            "Settlement service started."
        );

        Ok(SettlementServiceHandle { shutdown_tx: Some(shutdown_tx), task_handle })
    }
}

impl<P, N> std::fmt::Debug for SettlementService<P, N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettlementService")
            .field("backend", &self.pipeline.backend_name())
            .field("chain", &self.pipeline.chain_name())
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
    pipeline: Arc<dyn Settle>,
    batch_size: u64,
    idle_flush_interval: tokio::time::Duration,
    /// Last settled block, from the chain's core contract. `None` = nothing settled yet.
    cursor: Option<BlockNumber>,
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
                    error!(target: LOG_TARGET, %error, "Failed to read local chain head.");
                    tokio::time::sleep(RETRY_BACKOFF_MIN).await;
                    continue;
                }
            };

            let idle_elapsed = Instant::now() >= idle_deadline;

            match next_action(self.cursor, head, self.batch_size, idle_elapsed) {
                Action::Settle { first, last } => {
                    match self.settle_batch(first, last).await {
                        Ok(tx_hash) => {
                            info!(
                                target: LOG_TARGET,
                                first,
                                last,
                                %tx_hash,
                                "Settled block range."
                            );
                            self.cursor = Some(last);
                            idle_deadline = Instant::now() + self.idle_flush_interval;
                            backoff = RETRY_BACKOFF_MIN;
                            consecutive_failures = 0;
                            // Loop again immediately: drain any remaining backlog.
                        }

                        Err(error) => {
                            consecutive_failures += 1;
                            error!(
                                target: LOG_TARGET,
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
                            match self.pipeline.settled_block().await {
                                Ok(cursor) => {
                                    if cursor != self.cursor {
                                        warn!(
                                            target: LOG_TARGET,
                                            ?cursor,
                                            previous = ?self.cursor,
                                            "On-chain settlement cursor advanced despite the \
                                             error; continuing from it."
                                        );
                                        self.cursor = cursor;
                                    }
                                }
                                Err(error) => {
                                    error!(
                                        target: LOG_TARGET,
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

        info!(target: LOG_TARGET, "Settlement service stopped.");
    }

    /// Retrieve the latest block on the local chain.
    ///
    /// Errors if the chain has no blocks at all — which should not happen in normal operation,
    /// since the node commits the genesis block at startup. The run loop treats that error like
    /// any other transient read failure (log, back off, retry).
    fn local_head(&self) -> Result<BlockNumber> {
        self.provider.provider().latest_number().map_err(|e| e.into())
    }

    /// Prove and settle the inclusive block range `[first, last]`.
    async fn settle_batch(&self, first: BlockNumber, last: BlockNumber) -> Result<String> {
        let prev_block = if first == 0 { None } else { Some(first - 1) };
        self.pipeline.settle(prev_block, last).await
    }
}

#[cfg(test)]
mod tests {
    use super::{next_action, Action};

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
}
