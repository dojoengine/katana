use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::{Condvar, Mutex, RwLock};
use tracing::warn;

use crate::shard::{Shard, ShardId, ShardState};

pub type ShardRegistry = Arc<RwLock<HashMap<ShardId, Arc<Shard>>>>;

/// Shared task queue for scheduling shards that have pending transactions.
///
/// Workers block on [`next_task`](ShardScheduler::next_task) using a condvar,
/// so they must run on dedicated OS threads (not async tasks).
#[derive(Debug, Clone)]
pub struct Scheduler {
    inner: Arc<SchedulerInner>,
}

struct SchedulerInner {
    queue: Mutex<VecDeque<ShardId>>,
    shards: ShardRegistry,
    condvar: Condvar,
    time_quantum: Duration,
    shutdown: AtomicBool,
}

impl Scheduler {
    pub fn new(time_quantum: Duration) -> Self {
        Self {
            inner: Arc::new(SchedulerInner {
                queue: Mutex::new(VecDeque::new()),
                shards: Arc::new(RwLock::new(HashMap::new())),
                condvar: Condvar::new(),
                time_quantum,
                shutdown: AtomicBool::new(false),
            }),
        }
    }

    /// Returns the shared shard registry used by both manager and scheduler.
    pub fn shard_registry(&self) -> ShardRegistry {
        Arc::clone(&self.inner.shards)
    }

    /// Register a shard so it can be resolved by ID when scheduled.
    pub fn register_shard(&self, shard: Arc<Shard>) {
        self.inner.shards.write().insert(shard.id, shard);
    }

    /// Schedule a shard for processing. If the shard is already Pending or Running,
    /// this is a no-op to prevent double-scheduling.
    pub fn schedule(&self, id: ShardId) {
        let shard = {
            let shards = self.inner.shards.read();
            shards.get(&id).cloned()
        };

        let Some(shard) = shard else {
            warn!(%id, "attempted to schedule unknown shard id");
            return;
        };

        // Only schedule if the shard is currently Idle
        if shard.compare_exchange_state(ShardState::Idle, ShardState::Pending) {
            self.inner.queue.lock().push_back(id);
            self.inner.condvar.notify_one();
        }
    }

    /// Dequeue the next shard to process. Blocks the current thread for up to
    /// ~100 ms waiting for work. Returns `None` when no work is available.
    pub fn next_task(&self) -> Option<Arc<Shard>> {
        let id = {
            let mut queue = self.inner.queue.lock();
            if let id @ Some(..) = queue.pop_front() {
                id
            } else {
                // Block until notified (new work or shutdown wake) or timeout.
                self.inner.condvar.wait_for(&mut queue, Duration::from_millis(100));
                queue.pop_front()
            }
        };

        if let Some(id) = id {
            let shards = self.inner.shards.read();
            let shard = shards.get(&id).cloned();
            if shard.is_none() {
                warn!(%id, "scheduled shard id missing from registry");
            }
            shard
        } else {
            None
        }
    }

    /// Returns the time quantum for worker preemption.
    pub fn time_quantum(&self) -> Duration {
        self.inner.time_quantum
    }

    /// Returns the current number of scheduled shard tasks in the queue.
    pub fn queue_len(&self) -> usize {
        self.inner.queue.lock().len()
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
            .field("registered_shards", &self.shards.read().len())
            .field("time_quantum", &self.time_quantum)
            .finish_non_exhaustive()
    }
}
