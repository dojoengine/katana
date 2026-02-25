use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::{Condvar, Mutex};

use crate::shard::{Shard, ShardState};

/// Shared task queue for scheduling shards that have pending transactions.
///
/// Workers block on [`next_task`](ShardScheduler::next_task) using a condvar,
/// so they must run on dedicated OS threads (not async tasks).
#[derive(Debug, Clone)]
pub struct Scheduler {
    inner: Arc<SchedulerInner>,
}

struct SchedulerInner {
    queue: Mutex<VecDeque<Arc<Shard>>>,
    condvar: Condvar,
    time_quantum: Duration,
    shutdown: AtomicBool,
}

impl Scheduler {
    pub fn new(time_quantum: Duration) -> Self {
        Self {
            inner: Arc::new(SchedulerInner {
                queue: Mutex::new(VecDeque::new()),
                condvar: Condvar::new(),
                time_quantum,
                shutdown: AtomicBool::new(false),
            }),
        }
    }

    /// Schedule a shard for processing. If the shard is already Pending or Running,
    /// this is a no-op to prevent double-scheduling.
    pub fn schedule(&self, shard: Arc<Shard>) {
        // Only schedule if the shard is currently Idle
        if shard.compare_exchange_state(ShardState::Idle, ShardState::Pending) {
            self.inner.queue.lock().push_back(shard);
            self.inner.condvar.notify_one();
        }
    }

    /// Dequeue the next shard to process. Blocks the current thread for up to
    /// ~100 ms waiting for work. Returns `None` when no work is available.
    pub fn next_task(&self) -> Option<Arc<Shard>> {
        let mut queue = self.inner.queue.lock();

        if let shard @ Some(..) = queue.pop_front() {
            shard
        } else {
            // Block until notified (new work or shutdown wake) or timeout.
            self.inner.condvar.wait_for(&mut queue, Duration::from_millis(100));
            queue.pop_front()
        }
    }

    /// Returns the time quantum for worker preemption.
    pub fn time_quantum(&self) -> Duration {
        self.inner.time_quantum
    }

    /// Signal shutdown and wake all waiting workers.
    pub fn shutdown(&self) {
        self.inner.shutdown.store(true, Ordering::SeqCst);
        self.inner.condvar.notify_all();
    }

    /// Returns `true` if shutdown has been signaled.
    pub fn is_shutdown(&self) -> bool {
        self.inner.shutdown.load(Ordering::SeqCst)
    }
}

impl std::fmt::Debug for SchedulerInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SchedulerInner")
            .field("queue_len", &self.queue.lock().len())
            .field("time_quantum", &self.time_quantum)
            .finish_non_exhaustive()
    }
}
