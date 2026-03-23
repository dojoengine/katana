//! Message triggers.
//!
//! A trigger determines **when** the messenger should check for new messages.
//! It is a [`Stream`] that yields `()` each time it's appropriate to poll.

use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use futures::Stream;
use tokio::time::{interval_at, Instant, Interval};

/// A trigger that fires when it's time to check for new messages.
pub trait MessageTrigger: Stream<Item = ()> + Send + Unpin + 'static {}

/// Blanket impl — any matching `Stream` is a `MessageTrigger`.
impl<T> MessageTrigger for T where T: Stream<Item = ()> + Send + Unpin + 'static {}

/// A trigger that fires on a fixed time interval.
pub struct IntervalTrigger {
    interval: Interval,
}

impl IntervalTrigger {
    pub fn new(secs: u64) -> Self {
        let duration = Duration::from_secs(secs);
        let mut interval = interval_at(Instant::now() + duration, duration);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        Self { interval }
    }
}

impl Stream for IntervalTrigger {
    type Item = ();

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        match this.interval.poll_tick(cx) {
            Poll::Ready(_) => Poll::Ready(Some(())),
            Poll::Pending => Poll::Pending,
        }
    }
}
