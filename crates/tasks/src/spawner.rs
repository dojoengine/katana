use core::future::Future;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;

use futures::channel::oneshot;
use futures::FutureExt;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error};

use crate::{BlockingTaskHandle, Inner, JoinError, JoinHandle};

/// A builder for building tasks to be spawned on the associated task manager.
///
/// Can only be created using [`super::TaskSpawner::build_task`].
#[derive(Debug)]
pub struct TaskBuilder<'a> {
    /// The task manager that the task will be spawned on.
    spawner: &'a TaskSpawner,
    /// The name of the task.
    name: Option<String>,
    /// Notifies the task manager to perform a graceful shutdown when the task is finished due to
    /// completion or cancellation.
    graceful_shutdown: bool,

    critical: bool,
}

impl<'a> TaskBuilder<'a> {
    /// Creates a new task builder associated with the given task manager.
    pub(crate) fn new(spawner: &'a TaskSpawner) -> Self {
        Self { spawner, name: None, graceful_shutdown: false, critical: false }
    }

    /// Sets the name of the task.
    pub fn name<T: Into<String>>(mut self, name: T) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn graceful_shutdown(mut self) -> TaskBuilder<'a> {
        self.graceful_shutdown = true;
        self
    }

    /// Will perform graceful shutdown if the task panic.
    pub fn critical(mut self) -> TaskBuilder<'a> {
        self.critical = true;
        self
    }

    /// Spawns the given future based on the configured builder.
    pub fn spawn<F>(self, fut: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let task = self.build_task(fut);
        if self.critical {
            let critical_task = self.make_task_critical(task);
            self.spawner.spawn(critical_task)
        } else {
            self.spawner.spawn(task)
        }
    }

    /// Spawns an blocking task onto the Tokio runtime associated with the manager.
    ///
    /// # CPU-bound tasks
    ///
    /// If need to spawn a CPU-bound blocking task, consider using the
    /// [`CPUBoundTaskSpawner::spawn`] method instead.
    pub fn spawn_blocking<F, R>(&self, func: F) -> JoinHandle<R>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        self.spawner.spawn_blocking(self.build_blocking_task(func))
    }

    fn build_task<F>(&self, fut: F) -> impl Future<Output = F::Output>
    where
        F: Future + Send + 'static,
    {
        let task_name = self.name.clone();
        let graceful_shutdown = self.graceful_shutdown;
        let cancellation_token = self.spawner.cancellation_token().clone();

        // Creates a future that will send a cancellation signal to the manager when the future is
        // completed, regardless of success or error.
        fut.map(move |res| {
            if graceful_shutdown {
                debug!(target: "tasks", task = ?task_name, "Task with graceful shutdown completed.");
                cancellation_token.cancel();
            }
            res
        })
    }

    /// This method is intended to be called after the task has been built using the
    /// [`TaskBuilder::build_task`] method.
    fn make_task_critical<F>(&self, fut: F) -> impl Future<Output = F::Output>
    where
        F: Future + Send + 'static,
    {
        let task_name = self.name.clone();
        let cancellation_token = self.spawner.cancellation_token().clone();

        // Tokio already catches panics in the spawned task, but we are unable to handle it directly
        // inside the task without awaiting on it. So we catch it ourselves and resume it
        // again for tokio to catch it.
        AssertUnwindSafe(fut).catch_unwind().map(move |result| match result {
            Ok(value) => value,
            Err(error) => {
                let error_msg = match error.downcast_ref::<String>() {
                    None => "unknown",
                    Some(msg) => msg,
                };
                error!(error = %error_msg, task = ?task_name, "Critical task failed.");
                cancellation_token.cancel();
                std::panic::resume_unwind(error);
            }
        })
    }

    fn build_blocking_task<F, R>(&self, func: F) -> impl FnOnce() -> R + Send + 'static
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        let task_name = self.name.clone();
        let graceful_shutdown = self.graceful_shutdown;
        let cancellation_token = self.spawner.cancellation_token().clone();

        move || {
            let result = func();
            if graceful_shutdown {
                debug!(target: "tasks", task = ?task_name, "Task with graceful shutdown completed.");
                cancellation_token.cancel();
            }
            result
        }
    }
}

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
        let cancel_token = self.cancellation_token().clone();
        let task = cancellable(cancel_token, fut);

        let task = self.inner.tracker.track_future(task);
        let handle = self.inner.handle.spawn(task);

        JoinHandle(handle)
    }

    pub fn spawn_blocking<F, R>(&self, task: F) -> JoinHandle<R>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        let task = || crate::Result::Ok(task()); // blocking task is not cancellable
        let handle = self.inner.handle.spawn_blocking(task);
        JoinHandle(handle)
    }

    pub(crate) fn cancellation_token(&self) -> &CancellationToken {
        &self.inner.on_cancel
    }
}

/// A task spawner dedicated for spawning CPU-bound blocking tasks.
///
/// Tasks spawned by this spawner will be executed on a thread pool dedicated to CPU-bound tasks.
///
/// [`TaskManager`]: crate::manager::TaskManager
#[derive(Debug, Clone)]
pub struct CPUBoundTaskSpawner(TaskSpawner);

impl CPUBoundTaskSpawner {
    /// Spawns a CPU-bound blocking task onto the manager's blocking pool.
    pub fn spawn<F, R>(&self, func: F) -> BlockingTaskHandle<R>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        self.0.inner.blocking_pool.spawn(move || {
            let _ = tx.send(panic::catch_unwind(AssertUnwindSafe(func)));
        });
        BlockingTaskHandle(rx)
    }
}

async fn cancellable<F: Future>(
    cancellation_token: CancellationToken,
    fut: F,
) -> crate::Result<F::Output> {
    tokio::select! {
        res = fut => Ok(res),
        _ = cancellation_token.cancelled() => Err(JoinError::Cancelled)
    }
}

#[cfg(test)]
mod tests {

    use std::future::pending;

    use crate::TaskManager;

    #[tokio::test]
    async fn task_spawner_task_cancel() {
        let manager = TaskManager::current();
        let spawner = manager.task_spawner();

        // task will be running in the background
        let handle = spawner.spawn(pending::<()>());

        // cancel task using the cancellation token directly
        spawner.cancellation_token().cancel();
        let error = handle.await.unwrap_err();
        assert!(error.is_cancelled());

        let handle = spawner.spawn(pending::<()>());

        // cancel task using the abort method of the handle
        handle.abort();
        let error = handle.await.unwrap_err();
        assert!(error.is_cancelled());
    }

    #[tokio::test]
    async fn task_spawner_task_panic() {
        let manager = TaskManager::current();
        let spawner = manager.task_spawner();

        // non-blocking task

        let res = spawner.spawn(async { 1 + 1 }).await.unwrap();
        assert_eq!(res, 2);

        let result = spawner.spawn(async { panic!("test") }).await;
        let error = result.expect_err("should be panic error");
        assert!(error.is_panic());

        // blocking task

        let res = spawner.spawn_blocking(|| 1 + 1).await.unwrap();
        assert_eq!(res, 2);

        let result = spawner.spawn_blocking(|| panic!("test")).await;
        let error = result.expect_err("should be panic error");
        assert!(error.is_panic());
    }

    #[tokio::test]
    async fn cpu_bound_task_spawner_task_panic() {
        let manager = TaskManager::current();
        let spawner = manager.task_spawner().cpu_bound();

        let res = spawner.spawn(|| 1 + 1).await.unwrap();
        assert_eq!(res, 2);

        let result = spawner.spawn(|| panic!("test")).await;
        let error = result.expect_err("should be panic error");
        assert!(error.is_panic());
    }
}
