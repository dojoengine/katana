use std::collections::btree_set::IntoIter;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::{Stream, StreamExt};

use crate::ordering::PoolOrd;
use crate::subscription::Subscription;
use crate::tx::PendingTx;
use crate::PoolTransaction;

/// An iterator that yields transactions from the pool that can be included in a block, sorted by
/// by its priority.
#[derive(Debug)]
pub struct PendingTransactions<T, O: PoolOrd> {
    /// Iterator over all the pending transactions at the time of the creation of this struct.
    pub all: IntoIter<PendingTx<T, O>>,
    /// Subscription to the pool to get notified when new transactions are added. This is used to
    /// wait on the new transactions after exhausting the `all` iterator.
    pub subscription: Subscription<T, O>,
}

impl<T, O> Stream for PendingTransactions<T, O>
where
    T: PoolTransaction,
    O: PoolOrd<Transaction = T>,
{
    type Item = PendingTx<T, O>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if let Some(tx) = this.all.next() {
            Poll::Ready(Some(tx))
        } else {
            this.subscription.poll_next_unpin(cx)
        }
    }
}
