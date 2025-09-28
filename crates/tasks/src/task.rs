use std::any::Any;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::channel::oneshot;
use futures::FutureExt;
use tracing::{debug, error};

use crate::TaskSpawner;

#[derive(Debug)]
#[must_use = "JoinHandle does nothing unless polled"]
pub struct JoinHandle<T>(tokio::task::JoinHandle<TaskResult<T>>);

impl<T> JoinHandle<T> {
    pub(crate) fn new(handle: tokio::task::JoinHandle<TaskResult<T>>) -> Self {
        Self(handle)
    }

    /// Aborts the task associated with this handle.
    ///
    /// Be aware that tasks spawned using [`spawn_io_blocking`] cannot be aborted
    /// because they are not async. If you call `abort` on a `spawn_io_blocking`
    /// task, then this *will not have any effect*, and the task will continue
    /// running normally. The exception is if the task has not started running
    /// yet; in that case, calling `abort` may prevent the task from starting.
    pub fn abort(&self) {
        self.0.abort();
    }

    /// Returns `true` if the task has finished executing.
    pub fn is_finished(&self) -> bool {
        self.0.is_finished()
    }
}

impl<T> Future for JoinHandle<T> {
    type Output = TaskResult<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let inner = unsafe { self.map_unchecked_mut(|s| &mut s.0) };
        match inner.poll(cx) {
            // Panics in the spawned task are caught by Tokio. If the task panics, the `JoinError`
            // in the Err variant is the error that contains the panic. See the documentation for
            // `JoinHandle` for more information.
            Poll::Ready(Err(err)) => std::panic::resume_unwind(err.into_panic()),
            Poll::Ready(Ok(res)) => Poll::Ready(res),
            Poll::Pending => Poll::Pending,
        }
    }
}

type BlockingTaskResult<T> = Result<T, Box<dyn Any + Send>>;

#[derive(Debug)]
#[must_use = "BlockingTaskHandle does nothing unless polled"]
pub struct BlockingTaskHandle<T>(pub(crate) oneshot::Receiver<BlockingTaskResult<T>>);

impl<T> Future for BlockingTaskHandle<T> {
    type Output = TaskResult<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut self.get_mut().0).poll(cx) {
            Poll::Ready(Ok(result)) => match result {
                Ok(value) => Poll::Ready(TaskResult::Completed(value)),
                Err(err) => std::panic::resume_unwind(err),
            },
            Poll::Ready(Err(..)) => Poll::Ready(TaskResult::Cancelled),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// A task result that can be either completed or cancelled.
#[derive(Debug, Clone)]
pub enum TaskResult<T> {
    /// The task completed successfully with the given result.
    Completed(T),
    /// The task was cancelled.
    Cancelled,
}

impl<T> TaskResult<T> {
    /// Returns true if the task was cancelled.
    pub fn is_cancelled(&self) -> bool {
        matches!(self, TaskResult::Cancelled)
    }

    /// Unwraps the task result, panicking if it was cancelled.
    pub fn unwrap(self) -> T {
        match self {
            TaskResult::Completed(value) => value,
            TaskResult::Cancelled => panic!("Task was cancelled"),
        }
    }

    /// Unwraps the task result, panicking with a custom message if it was cancelled.
    pub fn expect(self, msg: &str) -> T {
        match self {
            TaskResult::Completed(value) => value,
            TaskResult::Cancelled => panic!("{msg}"),
        }
    }
}

#[derive(Debug)]
pub struct CriticalTask;

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
    /// ompletion or cancellation.
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
        let graceful_shutdown = self.graceful_shutdown;
        let cancellation_token = self.spawner.cancellation_token().clone();
        let task_name = self.name.clone().unwrap_or_else(|| "unnamed".to_string());

        // Creates a future that will send a cancellation signal to the manager when the future is
        // completed, regardless of success or error.
        fut.map(move |res| {
            if graceful_shutdown {
                debug!(target: "tasks", task = task_name, "Task with graceful shutdown completed.");
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
        let cancellation_token = self.spawner.cancellation_token().clone();
        let task_name = self.name.clone().unwrap_or_else(|| "unnamed".to_string());

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
                error!(error = %error_msg, task = task_name, "Critical task failed.");
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
        let graceful_shutdown = self.graceful_shutdown;
        let cancellation_token = self.spawner.cancellation_token().clone();
        let task_name = self.name.clone().unwrap_or_else(|| "unnamed".to_string());

        move || {
            let result = func();
            if graceful_shutdown {
                debug!(target: "tasks", task = task_name, "Task with graceful shutdown completed.");
                cancellation_token.cancel();
            }
            result
        }
    }
}
