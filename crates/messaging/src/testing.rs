//! Test doubles for the messaging component.
//!
//! Available inside the crate (`cfg(test)`) and to downstream crates via the
//! `testing` feature. Provides:
//! - [`MockCollector`] — a [`MessageCollector`] that returns canned responses and records its
//!   inputs, so tests can drive the state machine without an RPC.
//! - [`ManualTrigger`] — a [`MessageTrigger`] whose ticks are produced explicitly by a handle, so
//!   tests can control polling cadence deterministically.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::task::{Context, Poll};

use futures::Stream;
use katana_primitives::chain::ChainId;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use crate::stream::collector::{GatherResult, MessageCollector};
use crate::Error;

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
/// the method returns [`Error::GatherError`] so tests fail loudly when they
/// haven't enqueued enough responses.
///
/// All calls are recorded for assertions via [`latest_block_calls`] and
/// [`gather_calls`].
#[derive(Default)]
pub struct MockCollector {
    latest_block_responses: Mutex<VecDeque<Result<u64, Error>>>,
    gather_responses: Mutex<VecDeque<Result<GatherResult, Error>>>,
    latest_block_call_count: AtomicU64,
    gather_calls: Mutex<Vec<GatherCall>>,
}

impl MockCollector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a `latest_block` response onto the queue. Called in FIFO order.
    pub fn push_latest_block(&self, response: Result<u64, Error>) {
        self.latest_block_responses.lock().unwrap().push_back(response);
    }

    /// Push a `gather` response onto the queue. Called in FIFO order.
    pub fn push_gather(&self, response: Result<GatherResult, Error>) {
        self.gather_responses.lock().unwrap().push_back(response);
    }

    /// Number of times `latest_block` has been called so far.
    pub fn latest_block_calls(&self) -> u64 {
        self.latest_block_call_count.load(Ordering::SeqCst)
    }

    /// Snapshot of every `gather` call recorded so far.
    pub fn gather_calls(&self) -> Vec<GatherCall> {
        self.gather_calls.lock().unwrap().clone()
    }
}

impl MessageCollector for MockCollector {
    fn latest_block(&self) -> Pin<Box<dyn Future<Output = Result<u64, Error>> + Send + '_>> {
        self.latest_block_call_count.fetch_add(1, Ordering::SeqCst);
        let response = self
            .latest_block_responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Err(Error::GatherError));
        Box::pin(async move { response })
    }

    fn gather(
        &self,
        from_block: u64,
        from_tx_index: u64,
        to_block: u64,
        chain_id: ChainId,
    ) -> Pin<Box<dyn Future<Output = Result<GatherResult, Error>> + Send + '_>> {
        self.gather_calls.lock().unwrap().push(GatherCall {
            from_block,
            from_tx_index,
            to_block,
            chain_id,
        });
        let response =
            self.gather_responses.lock().unwrap().pop_front().unwrap_or(Err(Error::GatherError));
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
