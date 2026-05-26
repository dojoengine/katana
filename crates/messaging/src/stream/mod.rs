//! Unified message stream that composes a [`MessageCollector`] and a [`MessageTrigger`].

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::{Future, FutureExt, Stream, StreamExt};
use katana_primitives::block::BlockNumber;
use katana_primitives::chain::ChainId;
use tracing::{error, trace};

pub mod collector;
pub mod trigger;

use collector::{GatherResult, MessageCollector};
use trigger::MessageTrigger;

use crate::{MessagingOutcome, LOG_TARGET};

/// Maximum number of blocks to scan per gather call.
///
/// Gathering is chunked by block range instead of querying `[from_block, latest]`
/// in one shot because of how settlement RPCs bound a single query:
///
/// - Ethereum `eth_getLogs` has no pagination in the JSON-RPC spec (no continuation token), so an
///   over-large range fails wholesale with no way to resume mid-range. Providers cap a single query
///   well below all of history — commonly ~2k blocks or ~10k logs (Alchemy, Infura), with free
///   tiers far lower (Alchemy 10 blocks, QuickNode trial 5, Cloudflare 128). Chunking the range
///   client-side is the only portable way to stay under every cap.
/// - Starknet `starknet_getEvents` *does* paginate (continuation token), and the Starknet collector
///   already drains arbitrary ranges that way; there this cap only bounds per-gather memory and
///   work.
///
/// 200 is deliberately conservative: under every paid-tier block-range cap and
/// below typical log-count caps, trading extra round-trips for near-universal
/// provider compatibility. Catch-up speed doesn't depend on a larger value —
/// capped chunks are drained back-to-back without waiting for the trigger.
const MAX_BLOCKS_PER_GATHER: u64 = 200;

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// The phase of the stream's state machine.
enum MessageStreamPhase<E> {
    /// Waiting for the trigger to fire.
    Idle,
    /// Fetching the latest block number from the settlement chain.
    CheckingBlock(BoxFuture<Result<BlockNumber, E>>),
    /// Fetching messages from a known block range.
    Gathering {
        fut: BoxFuture<Result<GatherResult, E>>,
        /// `true` when this gather was capped by [`MAX_BLOCKS_PER_GATHER`] below
        /// the safe head — i.e. confirmed blocks remain past `to_block`. The
        /// stream re-checks and gathers the next chunk immediately instead of
        /// waiting for the trigger, so a far-behind cursor catches up at RPC
        /// speed rather than `MAX_BLOCKS_PER_GATHER` per trigger tick.
        more_pending: bool,
    },
}

/// A message stream that composes a collector ("how") and a trigger ("when").
///
/// On each trigger tick:
/// 1. Checks the latest settlement block via the collector.
/// 2. If new blocks exist, gathers messages from the block range.
/// 3. Yields a [`MessagingOutcome`] with the positioned messages.
///
/// The stream holds a resume cursor `(from_block, from_tx_index)`. After a successful
/// gather, `from_block` advances past `to_block` and `from_tx_index` resets to 0. The
/// cursor is passed to the collector on every gather so messages at or before the
/// cursor in `from_block` are filtered out (supports same-block resume after a crash).
#[allow(missing_debug_implementations)]
pub struct MessageStream<C: MessageCollector, T> {
    collector: Arc<C>,
    trigger: T,
    chain_id: ChainId,
    from_block: BlockNumber,
    from_tx_index: u64,
    /// Number of confirmations required before a settlement block is considered safe to
    /// gather from. The "safe head" is `latest_block - confirmation_depth`; gathers
    /// never cross past it. `0` disables the protection.
    confirmation_depth: u64,
    phase: MessageStreamPhase<C::Error>,
}

impl<C, T> MessageStream<C, T>
where
    C: MessageCollector,
    T: MessageTrigger,
{
    /// Create a new stream starting at `(from_block, 0)` with no confirmation depth.
    pub fn new(collector: C, trigger: T, chain_id: ChainId, from_block: u64) -> Self {
        Self::with_cursor(collector, trigger, chain_id, from_block, 0, 0)
    }

    /// Create a new stream starting at a specific `(from_block, from_tx_index)` cursor
    /// and a configurable confirmation depth.
    ///
    /// Used on restart when resuming from a persisted checkpoint that points mid-block.
    pub fn with_cursor(
        collector: C,
        trigger: T,
        chain_id: ChainId,
        from_block: u64,
        from_tx_index: u64,
        confirmation_depth: u64,
    ) -> Self {
        Self {
            collector: Arc::new(collector),
            trigger,
            chain_id,
            from_block,
            from_tx_index,
            confirmation_depth,
            phase: MessageStreamPhase::Idle,
        }
    }

    /// Returns the "safe head" — the highest settlement block that has accumulated
    /// enough confirmations to be considered immune to reorgs. Returns `None` if no
    /// block has yet reached that depth (i.e. `latest_block < confirmation_depth`).
    fn safe_head(&self, latest_block: u64) -> Option<u64> {
        latest_block.checked_sub(self.confirmation_depth)
    }

    /// Returns the capped `to_block` for a gather.
    fn to_block(from_block: u64, safe_head: u64) -> u64 {
        if from_block + MAX_BLOCKS_PER_GATHER + 1 < safe_head {
            from_block + MAX_BLOCKS_PER_GATHER
        } else {
            safe_head
        }
    }
}

impl<C, T> Stream for MessageStream<C, T>
where
    C: MessageCollector,
    T: MessageTrigger,
{
    type Item = MessagingOutcome;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        loop {
            match &mut this.phase {
                MessageStreamPhase::Idle => {
                    // Wait for the trigger to fire.
                    match this.trigger.poll_next_unpin(cx) {
                        Poll::Pending => return Poll::Pending,
                        Poll::Ready(Some(())) => {
                            let collector = this.collector.clone();
                            this.phase = MessageStreamPhase::CheckingBlock(Box::pin(async move {
                                collector.latest_block().await
                            }));
                        }
                        Poll::Ready(None) => return Poll::Ready(None),
                    }
                }

                MessageStreamPhase::CheckingBlock(fut) => match fut.poll_unpin(cx) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(Ok(latest_block)) => {
                        // Apply confirmation depth: only gather up to the safe head to
                        // avoid pulling messages from blocks that may still be reorg'd
                        // off the canonical chain.
                        let Some(safe_head) = this.safe_head(latest_block) else {
                            trace!(
                                target: LOG_TARGET,
                                from_block = this.from_block,
                                latest_block,
                                confirmation_depth = this.confirmation_depth,
                                "Settlement chain hasn't reached confirmation depth yet."
                            );

                            this.phase = MessageStreamPhase::Idle;
                            continue;
                        };

                        if safe_head < this.from_block {
                            trace!(
                                target: LOG_TARGET,
                                from_block = this.from_block,
                                latest_block,
                                safe_head,
                                "No new confirmed blocks on settlement chain."
                            );
                            this.phase = MessageStreamPhase::Idle;
                            // Loop back to Idle so the trigger gets re-polled and registers
                            // a waker for the next tick. Returning Pending without re-polling
                            // the trigger would leave the task with no waker, deadlocking
                            // the stream.
                            continue;
                        }

                        let to_block = Self::to_block(this.from_block, safe_head);
                        // Confirmed blocks remain past this gather's range when the
                        // chunk was capped below the safe head; drive the next chunk
                        // without waiting for the trigger.
                        let more_pending = to_block < safe_head;
                        trace!(
                            target: LOG_TARGET,
                            from_block = this.from_block,
                            from_tx_index = this.from_tx_index,
                            to_block,
                            latest_block,
                            safe_head,
                            more_pending,
                            "New blocks detected, gathering messages."
                        );

                        let collector = this.collector.clone();
                        let from_block = this.from_block;
                        let from_tx_index = this.from_tx_index;
                        let chain_id = this.chain_id;

                        this.phase = MessageStreamPhase::Gathering {
                            fut: Box::pin(async move {
                                collector
                                    .gather(from_block, from_tx_index, to_block, chain_id)
                                    .await
                            }),
                            more_pending,
                        };
                    }

                    Poll::Ready(Err(error)) => {
                        error!(target: LOG_TARGET, %error, "Failed to fetch latest block number.");
                        this.phase = MessageStreamPhase::Idle;
                        // re-poll the trigger so a waker is registered.
                        continue;
                    }
                },

                MessageStreamPhase::Gathering { fut, more_pending } => {
                    let more_pending = *more_pending;
                    match fut.poll_unpin(cx) {
                        Poll::Pending => return Poll::Pending,

                        Poll::Ready(Ok(result)) => {
                            // Advance cursor past the fully-inspected range. The server
                            // checkpoints per-message, so this bulk advance is safe: any
                            // crash between the cursor advance here and the next gather
                            // will re-gather this range and the server will skip already-
                            // processed messages via pool hash dedupe.
                            this.from_block = result.to_block + 1;
                            this.from_tx_index = 0;

                            // While still behind the safe head, re-check and gather the
                            // next chunk immediately rather than waiting for the trigger.
                            // Re-checking (vs. reusing a snapshot) keeps the safe head
                            // fresh as the chain advances during a long catch-up. Once a
                            // gather reaches the safe head, fall back to trigger polling.
                            if more_pending {
                                let collector = this.collector.clone();
                                this.phase =
                                    MessageStreamPhase::CheckingBlock(Box::pin(async move {
                                        collector.latest_block().await
                                    }));
                            } else {
                                this.phase = MessageStreamPhase::Idle;
                            }

                            trace!(
                                target: LOG_TARGET,
                                from_block = this.from_block,
                                from_tx_index = this.from_tx_index,
                                to_block = result.to_block,
                                messages_count = result.messages.len(),
                                more_pending,
                                "Messages gathered successfully."
                            );

                            return Poll::Ready(Some(MessagingOutcome {
                                settlement_block: result.to_block,
                                messages: result.messages,
                            }));
                        }

                        Poll::Ready(Err(error)) => {
                            error!(target: LOG_TARGET, block = %this.from_block, %error, "Gathering messages for block.");
                            this.phase = MessageStreamPhase::Idle;
                            // Re-poll the trigger so a waker is registered for the next tick.
                            continue;
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::task::{Context, Poll};
    use std::time::Duration;

    use futures::{Stream, StreamExt};
    use katana_primitives::chain::ChainId;
    use katana_primitives::transaction::L1HandlerTx;
    use katana_primitives::Felt;
    use parking_lot::Mutex;
    use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

    use super::collector::OrderedMessage;
    use super::*;
    use crate::stream::collector::{GatherResult, MessageCollector};

    const SHORT: Duration = Duration::from_millis(50);

    /// A dummy error type returned by [`MockCollector`] when a response queue is empty.
    #[derive(Debug, thiserror::Error)]
    #[error("mock collector error")]
    pub struct MockCollectorError;

    /// One recorded call to [`MockCollector::gather`].
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct GatherCall {
        pub from_block: u64,
        pub from_tx_index: u64,
        pub to_block: u64,
        pub chain_id: ChainId,
    }

    /// A [`MessageCollector`] backed by canned response queues.
    ///
    /// Each test pushes responses in expected order via [`push_latest_block`] and
    /// [`push_gather`]. The collector pops them on each call. If a queue is empty,
    /// the method returns [`MockCollectorError`] so tests fail loudly when they
    /// haven't enqueued enough responses.
    ///
    /// All calls are recorded for assertions via [`latest_block_calls`] and
    /// [`gather_calls`].
    #[derive(Default)]
    pub struct MockCollector {
        latest_block_responses: Mutex<VecDeque<Result<u64, MockCollectorError>>>,
        gather_responses: Mutex<VecDeque<Result<GatherResult, MockCollectorError>>>,
        latest_block_call_count: AtomicU64,
        gather_calls: Mutex<Vec<GatherCall>>,
    }

    impl MockCollector {
        pub fn new() -> Self {
            Self::default()
        }

        /// Push a `latest_block` response onto the queue. Called in FIFO order.
        pub fn push_latest_block(&self, response: Result<u64, MockCollectorError>) {
            self.latest_block_responses.lock().push_back(response);
        }

        /// Push a `gather` response onto the queue. Called in FIFO order.
        pub fn push_gather(&self, response: Result<GatherResult, MockCollectorError>) {
            self.gather_responses.lock().push_back(response);
        }

        /// Number of times `latest_block` has been called so far.
        pub fn latest_block_calls(&self) -> u64 {
            self.latest_block_call_count.load(Ordering::SeqCst)
        }

        /// Snapshot of every `gather` call recorded so far.
        pub fn gather_calls(&self) -> Vec<GatherCall> {
            self.gather_calls.lock().clone()
        }
    }

    impl MessageCollector for MockCollector {
        type Error = MockCollectorError;

        fn latest_block(
            &self,
        ) -> Pin<Box<dyn Future<Output = Result<u64, Self::Error>> + Send + '_>> {
            self.latest_block_call_count.fetch_add(1, Ordering::SeqCst);
            let response =
                self.latest_block_responses.lock().pop_front().unwrap_or(Err(MockCollectorError));
            Box::pin(async move { response })
        }

        fn gather(
            &self,
            from_block: u64,
            from_tx_index: u64,
            to_block: u64,
            chain_id: ChainId,
        ) -> Pin<Box<dyn Future<Output = Result<GatherResult, Self::Error>> + Send + '_>> {
            self.gather_calls.lock().push(GatherCall {
                from_block,
                from_tx_index,
                to_block,
                chain_id,
            });
            let response =
                self.gather_responses.lock().pop_front().unwrap_or(Err(MockCollectorError));
            Box::pin(async move { response })
        }
    }

    /// A [`MessageTrigger`] that fires only when [`ManualTriggerHandle::fire`] is called.
    ///
    /// Backed by an unbounded mpsc channel; the receiver side implements `Stream<Item=()>`.
    /// Dropping the handle ends the trigger stream, which lets tests observe how the
    /// messenger handles end-of-stream.
    pub struct ManualTrigger {
        rx: UnboundedReceiver<()>,
    }

    impl std::fmt::Debug for ManualTrigger {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("ManualTrigger").finish_non_exhaustive()
        }
    }

    /// Handle to fire ticks into a [`ManualTrigger`].
    #[derive(Debug, Clone)]
    pub struct ManualTriggerHandle {
        tx: UnboundedSender<()>,
    }

    impl ManualTrigger {
        pub fn new() -> (Self, ManualTriggerHandle) {
            let (tx, rx) = mpsc::unbounded_channel();
            (Self { rx }, ManualTriggerHandle { tx })
        }
    }

    impl ManualTriggerHandle {
        /// Fire a single tick. Returns `Ok(())` even after the trigger is dropped —
        /// tests rarely care about the difference; the messenger will see no further
        /// ticks either way.
        pub fn fire(&self) {
            let _ = self.tx.send(());
        }
    }

    impl Stream for ManualTrigger {
        type Item = ();

        fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            self.get_mut().rx.poll_recv(cx)
        }
    }

    /// Builds a stub `L1HandlerTx` whose internals don't matter for state machine
    /// tests — we only care that `MessageStream` plumbs values through correctly.
    fn stub_tx() -> L1HandlerTx {
        L1HandlerTx {
            calldata: vec![],
            chain_id: ChainId::default(),
            message_hash: Default::default(),
            paid_fee_on_l1: 0,
            nonce: Felt::ZERO,
            entry_point_selector: Felt::ZERO,
            version: Felt::ZERO,
            contract_address: Default::default(),
        }
    }

    fn msg(block: u64, tx_index: u64) -> OrderedMessage {
        OrderedMessage { block, tx_index, l1_tx_hash: [0u8; 32], tx: stub_tx() }
    }

    /// Build a stream wired to a fresh `MockCollector` + `ManualTrigger`.
    /// Returns the boxed stream alongside handles for queueing mock responses
    /// and firing trigger ticks.
    fn build(
        from_block: u64,
        from_tx_index: u64,
        confirmation_depth: u64,
    ) -> (
        Pin<Box<MessageStream<Arc<MockCollector>, ManualTrigger>>>,
        Arc<MockCollector>,
        ManualTriggerHandle,
    ) {
        let collector = Arc::new(MockCollector::new());
        let (trigger, handle) = ManualTrigger::new();
        let stream = Box::pin(MessageStream::with_cursor(
            collector.clone(),
            trigger,
            ChainId::default(),
            from_block,
            from_tx_index,
            confirmation_depth,
        ));

        (stream, collector, handle)
    }

    /// Drive the stream and assert it does NOT yield within `SHORT`.
    /// Used to verify "no new blocks" / "before confirmation depth" paths.
    async fn assert_no_yield<S: futures::Stream + Unpin>(stream: &mut S) {
        let res = tokio::time::timeout(SHORT, stream.next()).await;
        assert!(res.is_err(), "stream yielded when it shouldn't have");
    }

    // -------------------------------------------------------------------------
    // Happy path
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn yields_outcome_on_successful_gather() {
        let (mut stream, collector, trigger) = build(0, 0, 0);
        collector.push_latest_block(Ok(5));
        collector.push_gather(Ok(GatherResult { to_block: 5, messages: vec![msg(3, 0)] }));

        trigger.fire();
        let outcome = stream.next().await.expect("stream yielded");

        assert_eq!(outcome.settlement_block, 5);
        assert_eq!(outcome.messages.len(), 1);
        assert_eq!(outcome.messages[0].block, 3);
    }

    // -------------------------------------------------------------------------
    // Cursor: same-block resume on first gather, advance + reset on next
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn cursor_starts_at_with_cursor_then_advances_after_gather() {
        // Restart-from-checkpoint scenario: starts at (10, 7).
        // First gather: collector sees from_block=10, from_tx_index=7 (same-block resume).
        // After gather to to_block=20: cursor advances to (21, 0).
        // Second gather: collector sees from_block=21, from_tx_index=0.
        let (mut stream, collector, trigger) = build(10, 7, 0);
        collector.push_latest_block(Ok(20));
        collector.push_gather(Ok(GatherResult { to_block: 20, messages: vec![] }));

        trigger.fire();
        let _ = stream.next().await.expect("stream yielded");

        collector.push_latest_block(Ok(21));
        collector.push_gather(Ok(GatherResult { to_block: 21, messages: vec![] }));
        trigger.fire();
        let _ = stream.next().await.expect("stream yielded");

        let calls = collector.gather_calls();
        assert_eq!(calls.len(), 2);

        assert_eq!(calls[0].from_block, 10, "first call must use the with_cursor block");
        assert_eq!(calls[0].from_tx_index, 7, "first call must use the with_cursor tx_index");

        assert_eq!(calls[1].from_block, 21, "from_block must advance to to_block + 1");
        assert_eq!(calls[1].from_tx_index, 0, "from_tx_index must reset to 0 after gather");
    }

    // -------------------------------------------------------------------------
    // No new blocks / waker regression
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn no_new_blocks_does_not_yield_but_resumes_on_next_tick() {
        let (mut stream, collector, trigger) = build(10, 0, 0);

        // Tick 1: latest_block == 9, below from_block. No gather, no yield.
        collector.push_latest_block(Ok(9));
        trigger.fire();

        assert_no_yield(&mut stream).await;
        assert_eq!(collector.latest_block_calls(), 1);
        assert!(collector.gather_calls().is_empty());

        // Tick 2: latest_block == 11. Must wake and gather.
        collector.push_latest_block(Ok(11));
        collector.push_gather(Ok(GatherResult { to_block: 11, messages: vec![] }));
        trigger.fire();

        let outcome =
            tokio::time::timeout(SHORT, stream.next()).await.expect("woke up").expect("yielded");
        assert_eq!(outcome.settlement_block, 11);
    }

    // -------------------------------------------------------------------------
    // Confirmation depth (reorg protection)
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn confirmation_depth_caps_to_block() {
        // latest = 100, depth = 6 → safe_head = 94. Gather should be capped at 94.
        let (mut stream, collector, trigger) = build(0, 0, 6);
        collector.push_latest_block(Ok(100));
        collector.push_gather(Ok(GatherResult { to_block: 94, messages: vec![] }));

        trigger.fire();
        let outcome = stream.next().await.expect("yielded");
        assert_eq!(outcome.settlement_block, 94);

        let calls = collector.gather_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].to_block, 94, "to_block must respect confirmation_depth");
    }

    #[tokio::test]
    async fn no_gather_before_confirmation_depth_reached() {
        // latest_block < confirmation_depth: safe_head is None (the saturating_sub
        // branch). Distinct from `no_new_blocks_does_not_yield_but_resumes_on_next_tick`,
        // which exercises the safe_head-below-from_block branch.
        let (mut stream, collector, trigger) = build(0, 0, 100);
        collector.push_latest_block(Ok(5));
        trigger.fire();
        assert_no_yield(&mut stream).await;
        assert!(collector.gather_calls().is_empty());
    }

    // -------------------------------------------------------------------------
    // MAX_BLOCKS_PER_GATHER cap
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn to_block_capped_at_max_blocks_per_gather() {
        // from_block = 0, latest = 1000, depth = 0. Cap at 0 + MAX_BLOCKS_PER_GATHER.
        let (mut stream, collector, trigger) = build(0, 0, 0);
        collector.push_latest_block(Ok(1000));
        collector
            .push_gather(Ok(GatherResult { to_block: MAX_BLOCKS_PER_GATHER, messages: vec![] }));
        trigger.fire();
        let _ = stream.next().await.expect("yielded");

        let calls = collector.gather_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].to_block, MAX_BLOCKS_PER_GATHER);
    }

    // -------------------------------------------------------------------------
    // Catch-up: drain capped chunks back-to-back without waiting for the trigger
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn catches_up_back_to_back_without_waiting_for_trigger() {
        // from_block far behind a safe head of 1000 (depth 0). Each gather caps at
        // MAX_BLOCKS_PER_GATHER, so the stream must re-check and gather the next
        // chunk immediately rather than parking until the next trigger tick.
        let safe_head = 1000;
        let (mut stream, collector, trigger) = build(0, 0, 0);

        let m = MAX_BLOCKS_PER_GATHER;
        // (from_block, to_block) for three consecutive capped chunks.
        let chunks = [(0, m), (m + 1, 2 * m + 1), (2 * m + 2, 3 * m + 2)];
        for (_, to) in chunks {
            collector.push_latest_block(Ok(safe_head));
            collector.push_gather(Ok(GatherResult { to_block: to, messages: vec![] }));
        }

        // A SINGLE trigger tick.
        trigger.fire();

        // All three chunks must come through with no further trigger fires.
        for (_, to) in chunks {
            let outcome = tokio::time::timeout(SHORT, stream.next())
                .await
                .expect("chunk yielded without waiting for the trigger")
                .expect("stream yielded");
            assert_eq!(outcome.settlement_block, to);
        }

        let calls = collector.gather_calls();
        assert_eq!(calls.len(), 3, "three chunks drained from a single trigger tick");
        for (i, (from, to)) in chunks.iter().enumerate() {
            assert_eq!(calls[i].from_block, *from, "chunk {i} from_block");
            assert_eq!(calls[i].to_block, *to, "chunk {i} to_block");
        }
    }

    #[tokio::test]
    async fn stops_catching_up_at_safe_head_and_waits_for_trigger() {
        // safe_head = 150 < MAX_BLOCKS_PER_GATHER: the first gather reaches the safe
        // head in one (uncapped) chunk, so the stream must return to Idle and not
        // gather again until the next trigger fire.
        let (mut stream, collector, trigger) = build(0, 0, 0);
        collector.push_latest_block(Ok(150));
        collector.push_gather(Ok(GatherResult { to_block: 150, messages: vec![] }));

        trigger.fire();
        let outcome = stream.next().await.expect("stream yielded");
        assert_eq!(outcome.settlement_block, 150);

        // Even with the next chunk's responses queued, no gather happens until the
        // trigger fires again — proving catch-up stops at the safe head.
        collector.push_latest_block(Ok(300));
        collector.push_gather(Ok(GatherResult { to_block: 300, messages: vec![] }));
        assert_no_yield(&mut stream).await;
        assert_eq!(collector.gather_calls().len(), 1, "no further gather without a trigger tick");

        trigger.fire();
        let outcome = tokio::time::timeout(SHORT, stream.next())
            .await
            .expect("woke on the next trigger tick")
            .expect("stream yielded");
        assert_eq!(outcome.settlement_block, 300);
        assert_eq!(collector.gather_calls().len(), 2);
    }

    // -------------------------------------------------------------------------
    // Error recovery — both paths must re-poll the trigger
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn latest_block_error_does_not_advance_cursor_and_recovers() {
        let (mut stream, collector, trigger) = build(5, 0, 0);

        // Tick 1: latest_block errors. No gather. No yield. Cursor unchanged.
        collector.push_latest_block(Err(MockCollectorError));
        trigger.fire();
        assert_no_yield(&mut stream).await;
        assert!(collector.gather_calls().is_empty());

        // Tick 2: latest_block recovers. Gather called with original from_block.
        collector.push_latest_block(Ok(8));
        collector.push_gather(Ok(GatherResult { to_block: 8, messages: vec![] }));
        trigger.fire();
        let _ = tokio::time::timeout(SHORT, stream.next())
            .await
            .expect("woke after recovery")
            .expect("yielded");

        let calls = collector.gather_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].from_block, 5, "cursor must not advance on latest_block error");
    }

    #[tokio::test]
    async fn gather_error_does_not_advance_cursor_and_recovers() {
        let (mut stream, collector, trigger) = build(5, 0, 0);

        // Tick 1: gather errors. No yield. Cursor unchanged.
        collector.push_latest_block(Ok(8));
        collector.push_gather(Err(MockCollectorError));
        trigger.fire();
        assert_no_yield(&mut stream).await;

        // Tick 2: gather recovers from the same from_block.
        collector.push_latest_block(Ok(8));
        collector.push_gather(Ok(GatherResult { to_block: 8, messages: vec![] }));
        trigger.fire();
        let _ = tokio::time::timeout(SHORT, stream.next())
            .await
            .expect("woke after recovery")
            .expect("yielded");

        let calls = collector.gather_calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].from_block, 5);
        assert_eq!(calls[1].from_block, 5, "cursor must not advance on gather error");
    }

    // -------------------------------------------------------------------------
    // End-of-stream
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn stream_ends_when_trigger_closes() {
        let (mut stream, _collector, trigger) = build(0, 0, 0);
        drop(trigger); // closes the underlying channel
        let res = tokio::time::timeout(SHORT, stream.next()).await.expect("ready promptly");
        assert!(res.is_none(), "stream should terminate when trigger ends");
    }
}
