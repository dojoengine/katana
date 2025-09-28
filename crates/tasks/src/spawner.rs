use core::future::Future;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;

use futures::channel::oneshot;
use tokio_util::sync::CancellationToken;
use tracing::Span;

use crate::task::{TaskBuilder, TaskResult};
use crate::{BlockingTaskHandle, Inner, JoinHandle};

/// A spawner for spawning tasks on the [`TaskManager`] that it was derived from.
///
/// This is the main way to spawn tasks on a [`TaskManager`]. It can only be created
/// by calling [`TaskManager::task_spawner`].
#[derive(Debug, Clone)]
pub struct TaskSpawner {
    /// A handle to the [`TaskManager`] that this spawner is associated with.
    pub(crate) inner: Arc<Inner>,
}

impl TaskSpawner {
    /// Returns a new [`TaskBuilder`] for building a task.
    pub fn build_task(&self) -> TaskBuilder<'_> {
        TaskBuilder::new(self)
    }

    /// Returns a new [`CPUBoundTaskSpawner`] for spawningc CPU-bound tasks.
    pub fn cpu_bound(&self) -> CPUBoundTaskSpawner {
        CPUBoundTaskSpawner(self.clone())
    }

    pub fn spawn<F>(&self, fut: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let task = self.make_cancellable(fut);
        let task = self.inner.tracker.track_future(task);
        JoinHandle::new(self.inner.handle.spawn(task))
    }

    pub fn spawn_blocking<F, R>(&self, task: F) -> JoinHandle<R>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        let task = move || TaskResult::Completed(task());
        let handle = self.inner.handle.spawn_blocking(task);
        JoinHandle::new(handle)
    }

    pub(crate) fn cancellation_token(&self) -> &CancellationToken {
        &self.inner.on_cancel
    }

    fn make_cancellable<F>(&self, fut: F) -> impl Future<Output = TaskResult<F::Output>>
    where
        F: Future,
    {
        let ct = self.inner.on_cancel.clone();
        async move {
            tokio::select! {
                _ = ct.cancelled() => {
                    TaskResult::Cancelled
                },
                res = fut => {
                    TaskResult::Completed(res)
                },
            }
        }
    }
}

/// A task spawner dedicated for spawning CPU-bound blocking tasks onto the manager's blocking pool.
///
/// Tasks spawned by this spawner will be executed on [`TaskManager`]'s `rayon` threadpool.
///
/// [`TaskManager`]: crate::manager::TaskManager
#[derive(Debug)]
pub struct CPUBoundTaskSpawner(TaskSpawner);

impl CPUBoundTaskSpawner {
    /// Spawns a CPU-bound blocking task onto the manager's blocking pool.
    pub fn spawn<F, R>(&self, func: F) -> BlockingTaskHandle<R>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        let span = Span::current();

        self.0.inner.blocking_pool.spawn(move || {
            let _guard = span.enter();
            let _ = tx.send(panic::catch_unwind(AssertUnwindSafe(func)));
        });

        BlockingTaskHandle(rx)
    }

    /// Spawns an async function that is mostly CPU-bound blocking task onto the manager's blocking
    /// pool.
    pub fn spawn_async<F>(&self, future: F) -> BlockingTaskHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to build tokio runtime")
                .block_on(future)
        })
    }
}
