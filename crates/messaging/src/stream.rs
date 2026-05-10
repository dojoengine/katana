//! Unified message stream that composes a [`MessageCollector`] and a [`MessageTrigger`].

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::{Future, FutureExt, Stream, StreamExt};
use katana_primitives::chain::ChainId;
use tracing::{error, trace};

use crate::collector::{GatherResult, MessageCollector};
use crate::trigger::MessageTrigger;
use crate::{Error, MessagingOutcome, LOG_TARGET};

/// Maximum number of blocks to fetch in a single gather call.
const MAX_BLOCKS_PER_GATHER: u64 = 200;

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// The phase of the stream's state machine.
enum Phase {
    /// Waiting for the trigger to fire.
    Idle,
    /// Fetching the latest block number from the settlement chain.
    CheckingBlock(BoxFuture<Result<u64, Error>>),
    /// Fetching messages from a known block range.
    Gathering(BoxFuture<Result<GatherResult, Error>>),
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
pub struct MessageStream<C, T> {
    collector: Arc<C>,
    trigger: T,
    chain_id: ChainId,
    from_block: u64,
    from_tx_index: u64,
    /// Number of confirmations required before a settlement block is considered safe to
    /// gather from. The "safe head" is `latest_block - confirmation_depth`; gathers
    /// never cross past it. `0` disables the protection.
    confirmation_depth: u64,
    phase: Phase,
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
            phase: Phase::Idle,
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
                Phase::Idle => {
                    // Wait for the trigger to fire.
                    match this.trigger.poll_next_unpin(cx) {
                        Poll::Ready(Some(())) => {
                            let collector = this.collector.clone();
                            this.phase = Phase::CheckingBlock(Box::pin(async move {
                                collector.latest_block().await
                            }));
                        }
                        Poll::Ready(None) => return Poll::Ready(None),
                        Poll::Pending => return Poll::Pending,
                    }
                }

                Phase::CheckingBlock(fut) => match fut.poll_unpin(cx) {
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
                            this.phase = Phase::Idle;
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
                            this.phase = Phase::Idle;
                            // Loop back to Idle so the trigger gets re-polled and registers
                            // a waker for the next tick. Returning Pending without re-polling
                            // the trigger would leave the task with no waker, deadlocking
                            // the stream.
                            continue;
                        }

                        let to_block = Self::to_block(this.from_block, safe_head);
                        trace!(
                            target: LOG_TARGET,
                            from_block = this.from_block,
                            from_tx_index = this.from_tx_index,
                            to_block,
                            latest_block,
                            safe_head,
                            "New blocks detected, gathering messages."
                        );

                        let collector = this.collector.clone();
                        let from_block = this.from_block;
                        let from_tx_index = this.from_tx_index;
                        let chain_id = this.chain_id;
                        this.phase = Phase::Gathering(Box::pin(async move {
                            collector.gather(from_block, from_tx_index, to_block, chain_id).await
                        }));
                    }
                    Poll::Ready(Err(e)) => {
                        error!(target: LOG_TARGET, error = %e, "Failed to fetch latest block number.");
                        this.phase = Phase::Idle;
                        // Same reason as above: re-poll the trigger so a waker is registered.
                        continue;
                    }
                    Poll::Pending => return Poll::Pending,
                },

                Phase::Gathering(fut) => match fut.poll_unpin(cx) {
                    Poll::Ready(Ok(result)) => {
                        // Advance cursor past the fully-inspected range. The server
                        // checkpoints per-message, so this bulk advance is safe: any
                        // crash between the cursor advance here and the next gather
                        // will re-gather this range and the server will skip already-
                        // processed messages via pool hash dedupe.
                        this.from_block = result.to_block + 1;
                        this.from_tx_index = 0;
                        this.phase = Phase::Idle;
                        return Poll::Ready(Some(MessagingOutcome {
                            settlement_block: result.to_block,
                            messages: result.messages,
                        }));
                    }
                    Poll::Ready(Err(e)) => {
                        error!(
                            target: LOG_TARGET,
                            block = %this.from_block,
                            error = %e,
                            "Gathering messages for block."
                        );
                        this.phase = Phase::Idle;
                        // Re-poll the trigger so a waker is registered for the next tick.
                        continue;
                    }
                    Poll::Pending => return Poll::Pending,
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures::StreamExt;
    use katana_primitives::chain::ChainId;
    use katana_primitives::transaction::L1HandlerTx;
    use katana_primitives::Felt;

    use super::*;
    use crate::collector::PositionedMessage;
    use crate::testing::{ManualTrigger, MockCollector};

    const SHORT: Duration = Duration::from_millis(50);

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

    fn msg(block: u64, tx_index: u64) -> PositionedMessage {
        PositionedMessage { block, tx_index, l1_tx_hash: [0u8; 32], tx: stub_tx() }
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
        crate::testing::ManualTriggerHandle,
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

    #[tokio::test]
    async fn empty_gather_still_yields_outcome() {
        // The server expects to see every gather (for logging / counters); empty
        // outcomes are fine but mustn't be swallowed.
        let (mut stream, collector, trigger) = build(0, 0, 0);
        collector.push_latest_block(Ok(2));
        collector.push_gather(Ok(GatherResult { to_block: 2, messages: vec![] }));

        trigger.fire();
        let outcome = stream.next().await.expect("stream yielded");

        assert_eq!(outcome.settlement_block, 2);
        assert!(outcome.messages.is_empty());
    }

    // -------------------------------------------------------------------------
    // Cursor advance
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn cursor_advances_past_to_block_after_gather() {
        let (mut stream, collector, trigger) = build(10, 7, 0);
        collector.push_latest_block(Ok(20));
        collector.push_gather(Ok(GatherResult { to_block: 20, messages: vec![] }));

        trigger.fire();
        let _ = stream.next().await.expect("stream yielded");

        // Next tick should ask the collector with from_block = 21, from_tx_index = 0.
        collector.push_latest_block(Ok(21));
        collector.push_gather(Ok(GatherResult { to_block: 21, messages: vec![] }));
        trigger.fire();
        let _ = stream.next().await.expect("stream yielded");

        let calls = collector.gather_calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1].from_block, 21);
        assert_eq!(calls[1].from_tx_index, 0, "tx_index should reset to 0 after gather");
    }

    #[tokio::test]
    async fn with_cursor_passes_from_tx_index_to_collector() {
        // Restart-from-checkpoint scenario: same-block resume.
        let (mut stream, collector, trigger) = build(50, 7, 0);
        collector.push_latest_block(Ok(50));
        collector.push_gather(Ok(GatherResult { to_block: 50, messages: vec![] }));

        trigger.fire();
        let _ = stream.next().await.expect("stream yielded");

        let calls = collector.gather_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].from_block, 50);
        assert_eq!(calls[0].from_tx_index, 7);
        assert_eq!(calls[0].to_block, 50);
    }

    // -------------------------------------------------------------------------
    // No new blocks / waker regression
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn no_new_blocks_does_not_yield_but_resumes_on_next_tick() {
        // Regression for the waker bug: previously after seeing "no new blocks"
        // the stream returned Pending without re-polling the trigger, so the
        // next tick never woke the task.
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
        // latest_block < confirmation_depth: safe_head is None, no gather.
        let (mut stream, collector, trigger) = build(0, 0, 100);
        collector.push_latest_block(Ok(5));
        trigger.fire();
        assert_no_yield(&mut stream).await;
        assert!(collector.gather_calls().is_empty());
    }

    #[tokio::test]
    async fn no_gather_when_safe_head_below_from_block() {
        // latest = 10, depth = 6 → safe_head = 4. from_block = 5 → 4 < 5, no gather.
        let (mut stream, collector, trigger) = build(5, 0, 6);
        collector.push_latest_block(Ok(10));
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
        collector.push_gather(Ok(GatherResult {
            to_block: MAX_BLOCKS_PER_GATHER,
            messages: vec![],
        }));
        trigger.fire();
        let _ = stream.next().await.expect("yielded");

        let calls = collector.gather_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].to_block, MAX_BLOCKS_PER_GATHER);
    }

    // -------------------------------------------------------------------------
    // Error recovery — both paths must re-poll the trigger
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn latest_block_error_does_not_advance_cursor_and_recovers() {
        let (mut stream, collector, trigger) = build(5, 0, 0);

        // Tick 1: latest_block errors. No gather. No yield. Cursor unchanged.
        collector.push_latest_block(Err(Error::GatherError));
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
        collector.push_gather(Err(Error::GatherError));
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

