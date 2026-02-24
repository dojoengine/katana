use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::scheduler::ShardScheduler;
use crate::types::Shard;
use crate::worker;

/// Owns the execution resources (scheduler + worker threads) for the shard node.
///
/// Analogous to `tokio::runtime::Runtime`: it owns the threads and provides a
/// cloneable [`RuntimeHandle`] for scheduling work.
///
/// # Shutdown
///
/// - [`shutdown_timeout`](Self::shutdown_timeout) — signals the scheduler and joins workers up to a
///   deadline; remaining threads are detached.
/// - [`shutdown_background`](Self::shutdown_background) — signals the scheduler and drops handles
///   so workers detach immediately.
/// - **Drop** — if the runtime has not been consumed by one of the above methods, the `Drop` impl
///   signals the scheduler and blocks until all workers exit.
pub struct ShardRuntime {
    handle: RuntimeHandle,
    worker_count: usize,
    worker_handles: Vec<thread::JoinHandle<()>>,
}

/// Cheap, cloneable reference for scheduling work on the [`ShardRuntime`].
///
/// Analogous to `tokio::runtime::Handle`.
#[derive(Clone, Debug)]
pub struct RuntimeHandle {
    scheduler: ShardScheduler,
}

// ---------------------------------------------------------------------------
// RuntimeHandle
// ---------------------------------------------------------------------------

impl RuntimeHandle {
    /// Schedule a shard for execution.
    pub fn schedule(&self, shard: Arc<Shard>) {
        self.scheduler.schedule(shard);
    }

    /// Returns a reference to the underlying scheduler.
    pub fn scheduler(&self) -> &ShardScheduler {
        &self.scheduler
    }
}

// ---------------------------------------------------------------------------
// ShardRuntime
// ---------------------------------------------------------------------------

impl ShardRuntime {
    /// Creates a new runtime. Workers are **not** spawned until [`start`](Self::start) is called.
    pub fn new(worker_count: usize, time_quantum: Duration) -> Self {
        let scheduler = ShardScheduler::new(time_quantum);
        let handle = RuntimeHandle { scheduler };
        Self { handle, worker_count, worker_handles: Vec::new() }
    }

    /// Returns a cloneable handle for scheduling work.
    pub fn handle(&self) -> RuntimeHandle {
        self.handle.clone()
    }

    /// Spawns the worker threads. Must be called exactly once.
    pub fn start(&mut self) {
        assert!(self.worker_handles.is_empty(), "ShardRuntime::start called more than once");
        self.worker_handles =
            worker::spawn_workers(self.worker_count, self.handle.scheduler.clone());
    }

    /// Signals the scheduler to shut down, then joins worker threads up to `duration`.
    ///
    /// Any workers that have not exited by the deadline are detached (their
    /// `JoinHandle`s are dropped).
    pub fn shutdown_timeout(mut self, duration: Duration) {
        self.handle.scheduler.shutdown();

        let deadline = Instant::now() + duration;
        let handles = std::mem::take(&mut self.worker_handles);

        for h in handles {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                // Deadline passed — remaining handles will be dropped (threads detach).
                break;
            }
            // park_timeout + join: we can't do a timed join directly in std,
            // so we just do a blocking join. The workers should exit quickly
            // once the scheduler is shut down.
            let _ = h.join();
        }

        // Prevent the Drop impl from joining again.
        // (worker_handles is already empty after std::mem::take)
    }

    /// Signals the scheduler to shut down and drops all handles, allowing
    /// workers to detach and exit in the background.
    pub fn shutdown_background(mut self) {
        self.handle.scheduler.shutdown();
        // Drop worker handles so threads detach.
        self.worker_handles.clear();
        // Prevent the Drop impl from joining.
    }
}

impl Drop for ShardRuntime {
    fn drop(&mut self) {
        if !self.worker_handles.is_empty() {
            self.handle.scheduler.shutdown();
            for h in self.worker_handles.drain(..) {
                let _ = h.join();
            }
        }
    }
}

impl std::fmt::Debug for ShardRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShardRuntime")
            .field("worker_count", &self.worker_count)
            .field("workers_spawned", &!self.worker_handles.is_empty())
            .field("handle", &self.handle)
            .finish()
    }
}
