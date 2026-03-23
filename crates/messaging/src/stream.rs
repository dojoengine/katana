//! Unified message stream that composes a [`MessageCollector`] and a [`MessageTrigger`].

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::{Future, FutureExt, Stream, StreamExt};
use katana_primitives::chain::ChainId;
use katana_primitives::transaction::L1HandlerTx;
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
/// 3. Yields a [`MessagingOutcome`] with the gathered transactions.
#[allow(missing_debug_implementations)]
pub struct MessageStream<C, T> {
    collector: Arc<C>,
    trigger: T,
    chain_id: ChainId,
    from_block: u64,
    phase: Phase,
}

impl<C, T> MessageStream<C, T>
where
    C: MessageCollector,
    T: MessageTrigger,
{
    pub fn new(collector: C, trigger: T, chain_id: ChainId, from_block: u64) -> Self {
        Self {
            collector: Arc::new(collector),
            trigger,
            chain_id,
            from_block,
            phase: Phase::Idle,
        }
    }

    /// Returns the capped `to_block` for a gather.
    fn to_block(from_block: u64, latest_block: u64) -> u64 {
        if from_block + MAX_BLOCKS_PER_GATHER + 1 < latest_block {
            from_block + MAX_BLOCKS_PER_GATHER
        } else {
            latest_block
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
                            this.phase =
                                Phase::CheckingBlock(Box::pin(async move {
                                    collector.latest_block().await
                                }));
                        }
                        Poll::Ready(None) => return Poll::Ready(None),
                        Poll::Pending => return Poll::Pending,
                    }
                }

                Phase::CheckingBlock(fut) => match fut.poll_unpin(cx) {
                    Poll::Ready(Ok(latest_block)) => {
                        if latest_block < this.from_block {
                            trace!(
                                target: LOG_TARGET,
                                from_block = this.from_block,
                                latest_block,
                                "No new blocks on settlement chain."
                            );
                            this.phase = Phase::Idle;
                            return Poll::Pending;
                        }

                        let to_block = Self::to_block(this.from_block, latest_block);
                        trace!(
                            target: LOG_TARGET,
                            from_block = this.from_block,
                            to_block,
                            latest_block,
                            "New blocks detected, gathering messages."
                        );

                        let collector = this.collector.clone();
                        let from_block = this.from_block;
                        let chain_id = this.chain_id;
                        this.phase = Phase::Gathering(Box::pin(async move {
                            collector.gather(from_block, to_block, chain_id).await
                        }));
                    }
                    Poll::Ready(Err(e)) => {
                        error!(target: LOG_TARGET, error = %e, "Failed to fetch latest block number.");
                        this.phase = Phase::Idle;
                        return Poll::Pending;
                    }
                    Poll::Pending => return Poll::Pending,
                },

                Phase::Gathering(fut) => match fut.poll_unpin(cx) {
                    Poll::Ready(Ok(result)) => {
                        this.from_block = result.to_block + 1;
                        this.phase = Phase::Idle;
                        return Poll::Ready(Some(MessagingOutcome {
                            settlement_block: result.to_block,
                            tx_index: result.tx_index,
                            transactions: result.transactions,
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
                        return Poll::Pending;
                    }
                    Poll::Pending => return Poll::Pending,
                },
            }
        }
    }
}
