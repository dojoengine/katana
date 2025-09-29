use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};
use std::sync::Arc;

use futures::future::BoxFuture;
use futures::FutureExt;
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;
pub use tokio_util::sync::WaitForCancellationFuture as WaitForShutdownFuture;
use tokio_util::task::TaskTracker;
use tracing::trace;

use crate::{CpuBlockingTaskPool, TaskSpawner};

/// Usage for this task manager is mainly to spawn tasks that can be cancelled, and capture
/// panicked tasks (which in the context of the task manager are considered critical) for graceful
/// shutdown.
///
/// When dispatching blocking work, prefer [`TaskManager::spawn_blocking`] for CPU-bound jobs (it
/// runs on a dedicated Rayon pool) and [`TaskManager::spawn_io_blocking`] for IO-heavy operations
/// that should stay within Tokio's `spawn_blocking` executor. Refer to the [CPU-bound tasks and
/// blocking code] section of the *tokio* docs and this [blog post] for more information.
///
/// # Spawning tasks
///
/// To spawn tasks on the manager, call [`TaskManager::task_spawner`] to get a [`TaskSpawner`]
/// instance. The [`TaskSpawner`] can then be used to spawn tasks on the manager.
///
/// # Tasks cancellation
///
/// When the manager is dropped, all tasks that have yet to complete will be cancelled.
///
/// [CPU-bound tasks and blocking code]: https://docs.rs/tokio/latest/tokio/index.html#cpu-bound-tasks-and-blocking-code
/// [blog post]: https://ryhl.io/blog/async-what-is-blocking/
#[derive(Debug, Clone)]
pub struct TaskManager {
    inner: Arc<Inner>,
}

#[derive(Debug)]
pub(crate) struct Inner {
    /// A handle to the Tokio runtime.
    pub(crate) handle: Handle,
    /// Keep track of currently running tasks.
    pub(crate) tracker: TaskTracker,
    /// Used to cancel all running tasks.
    ///
    /// This is passed to all the tasks spawned by the manager.
    pub(crate) on_cancel: CancellationToken,
    /// Pool dedicated to CPU-bound blocking work.
    pub(crate) blocking_pool: CpuBlockingTaskPool,
}

impl TaskManager {
    /// Create a new [`TaskManager`] from the given Tokio runtime handle using the default blocking
    /// pool configuration.
    pub fn new(handle: Handle) -> Self {
        let blocking_pool = CpuBlockingTaskPool::builder()
            .thread_name(|i| format!("blocking-thread-pool-{i}"))
            .build()
            .expect("failed to build blocking task thread pool");

        Self {
            inner: Arc::new(Inner {
                handle,
                blocking_pool,
                tracker: TaskTracker::new(),
                on_cancel: CancellationToken::new(),
            }),
        }
    }

    /// Create a new [`TaskManager`] from the ambient Tokio runtime using the default blocking pool
    /// configuration.
    ///
    /// # Panics
    ///
    /// This will panic if called outside the context of a Tokio runtime. That means that you must
    /// call this on one of the threads being run by the runtime,
    pub fn current() -> Self {
        Self::new(Handle::current())
    }

    /// Returns a [`TaskSpawner`] that can be used to spawn tasks onto the current [`TaskManager`].
    pub fn task_spawner(&self) -> TaskSpawner {
        TaskSpawner { inner: Arc::clone(&self.inner) }
    }

    /// Returns a future that can be awaited for the shutdown signal to be received.
    pub fn wait_for_shutdown(&self) -> WaitForShutdownFuture<'_> {
        self.inner.on_cancel.cancelled()
    }

    /// Shuts down the manager and wait until all currently running tasks are finished, either due
    /// to completion or cancellation.
    ///
    /// No task can be spawned on the manager after this method is called.
    pub fn shutdown(&self) -> ShutdownFuture<'_> {
        let fut = Box::pin(async {
            if !self.inner.on_cancel.is_cancelled() {
                self.inner.on_cancel.cancel();
            }

            self.wait_for_shutdown().await;

            // need to close the tracker first before waiting
            let _ = self.inner.tracker.close();
            self.inner.tracker.wait().await;
        });

        ShutdownFuture { fut }
    }

    /// Wait until all spawned tasks are completed.
    #[cfg(test)]
    async fn wait(&self) {
        let _ = self.inner.tracker.close();
        self.inner.tracker.wait().await;
        let _ = self.inner.tracker.reopen();
    }
}

/// A futures that resolves when the [`TaskManager`] is shutdown.
#[must_use = "futures do nothing unless polled"]
pub struct ShutdownFuture<'a> {
    fut: BoxFuture<'a, ()>,
}

impl Future for ShutdownFuture<'_> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.get_mut().fut.poll_unpin(cx)
    }
}

impl core::fmt::Debug for ShutdownFuture<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ShutdownFuture").field("fut", &"...").finish()
    }
}

impl Drop for TaskManager {
    fn drop(&mut self) {
        trace!(target: "tasks", "Task manager is dropped, cancelling all ongoing tasks.");
        self.inner.on_cancel.cancel();
    }
}

#[cfg(test)]
mod tests {

    use std::future::pending;

    use futures::future;
    use tokio::time::{self, Duration};

    use super::*;

    #[tokio::test]
    async fn normal_tasks() {
        let manager = TaskManager::current();
        let spawner = manager.task_spawner();

        let _ = spawner.build_task().spawn(time::sleep(Duration::from_millis(10)));
        let _ = spawner.build_task().spawn(time::sleep(Duration::from_millis(10)));
        assert_eq!(manager.inner.tracker.len(), 2);

        manager.wait().await;

        assert!(
            !manager.inner.on_cancel.is_cancelled(),
            "cancellation signal shouldn't be sent on normal task completion"
        )
    }

    #[tokio::test]
    async fn task_graceful_shutdown_on_success_completion() {
        let manager = TaskManager::current();
        let spawner = manager.task_spawner();

        let task1 = spawner.build_task().spawn(pending::<()>());
        let task2 = spawner.build_task().spawn(pending::<()>());
        assert!(!manager.inner.on_cancel.is_cancelled(), "should be cancelled yet");

        // normal task completion shouldn't trigger graceful shutdown
        let _ = spawner.build_task().spawn(future::ready(())).await;
        assert!(!manager.inner.on_cancel.is_cancelled());

        // but we can still spawn new tasks, and ongoing tasks shouldn't be cancelled
        let result = spawner.spawn(async { true }).await;
        assert!(result.is_ok());

        // task with graceful shutdown should trigger graceful shutdown on success completion
        let _ = spawner.build_task().graceful_shutdown().spawn(future::ready(())).await;
        assert!(manager.inner.on_cancel.is_cancelled());

        // wait for the task manager to shutdown gracefully
        manager.wait_for_shutdown().await;

        // all running tasks should be cancelled
        let task1_result = task1.await.unwrap_err();
        let task2_result = task2.await.unwrap_err();

        assert!(task1_result.is_cancelled());
        assert!(task2_result.is_cancelled());
    }

    #[tokio::test]
    async fn task_graceful_shutdown_on_panicked() {
        let manager = TaskManager::current();
        let spawner = manager.task_spawner();

        let task1 = spawner.build_task().spawn(pending::<()>());
        let task2 = spawner.build_task().spawn(pending::<()>());
        assert!(!manager.inner.on_cancel.is_cancelled(), "should be cancelled yet");

        // normal task panicking shouldn't trigger graceful shutdown
        let result = spawner.build_task().spawn(async { panic!("panicking") }).await;
        assert!(result.unwrap_err().is_panic());
        assert!(!manager.inner.on_cancel.is_cancelled());

        // but we can still spawn new tasks, and ongoing tasks shouldn't be cancelled
        let result = spawner.spawn(async { true }).await;
        assert!(result.is_ok());

        // task with graceful shutdown should trigger graceful shutdown on panic
        let result =
            spawner.build_task().graceful_shutdown().spawn(async { panic!("panicking") }).await;
        assert!(result.unwrap_err().is_panic());

        // wait for the task manager to shutdown gracefully
        manager.wait_for_shutdown().await;

        // all running tasks should be cancelled
        let task1_result = task1.await.unwrap_err();
        let task2_result = task2.await.unwrap_err();

        assert!(task1_result.is_cancelled());
        assert!(task2_result.is_cancelled());
    }

    #[tokio::test]
    async fn blocking_task_graceful_shutdown_on_panicked() {
        let manager = TaskManager::current();
        let spawner = manager.task_spawner();

        let task1 = spawner.build_task().spawn(pending::<()>());
        let task2 = spawner.build_task().spawn(pending::<()>());
        assert!(!manager.inner.on_cancel.is_cancelled(), "should be cancelled yet");

        // normal blocking task panicking shouldn't trigger graceful shutdown
        let result = spawner.build_task().spawn_blocking(|| panic!("panicking")).await;
        assert!(result.unwrap_err().is_panic());
        assert!(!manager.inner.on_cancel.is_cancelled());

        // but we can still spawn new tasks, and ongoing tasks shouldn't be cancelled
        let result = spawner.spawn_blocking(|| true).await;
        assert!(result.is_ok());

        // blocking task with graceful shutdown should trigger graceful shutdown on panic
        let result =
            spawner.build_task().graceful_shutdown().spawn_blocking(|| panic!("panicking")).await;
        assert!(result.unwrap_err().is_panic());

        // wait for the task manager to shutdown gracefully
        manager.wait_for_shutdown().await;

        // all running tasks should be cancelled
        let task1_result = task1.await.unwrap_err();
        let task2_result = task2.await.unwrap_err();

        assert!(task1_result.is_cancelled());
        assert!(task2_result.is_cancelled());
    }
}
