use std::any::Any;
use std::future::Future;
use std::marker::PhantomData;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::future::Either;
use futures::{FutureExt, TryFutureExt};
use thiserror::Error;
use tokio::task::JoinHandle;
use tokio_metrics::TaskMonitor;
use tracing::{debug, error};

use crate::TaskSpawner;

#[derive(Debug)]
#[must_use = "TaskHandle does nothing unless polled"]
pub struct TaskHandle<T>(JoinHandle<TaskResult<T>>);

impl<T> TaskHandle<T> {
    pub(crate) fn new(inner: JoinHandle<TaskResult<T>>) -> Self {
        Self(inner)
    }

    /// Aborts the task associated with this handle.
    pub fn abort(&self) {
        self.0.abort();
    }

    /// Returns `true` if the task has finished executing.
    pub fn is_finished(&self) -> bool {
        self.0.is_finished()
    }
}

impl<T> Future for TaskHandle<T> {
    type Output = TaskResult<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let inner = unsafe { self.map_unchecked_mut(|s| &mut s.0) };
        match inner.poll(cx) {
            Poll::Ready(Ok(res)) => Poll::Ready(res),
            Poll::Ready(Err(err)) => std::panic::resume_unwind(err.into_panic()),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// A task result that can be either completed or cancelled.
#[derive(Debug, Copy, Clone)]
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
}

#[derive(Debug)]
pub struct CriticalTask;

/// A builder for building tasks to be spawned on the associated task manager.
///
/// Can only be created using [`super::TaskSpawner::build_task`].
#[derive(Debug)]
pub struct TaskBuilder<'a, T = ()> {
    /// The task manager that the task will be spawned on.
    spawner: &'a TaskSpawner,
    /// The name of the task.
    name: Option<String>,
    /// Indicates whether the task should be instrumented.
    instrument: bool,
    /// Notifies the task manager to perform a graceful shutdown when the task is finished due to
    /// ompletion or cancellation.
    graceful_shutdown: bool,

    _phantom: PhantomData<T>,
}

impl<'a> TaskBuilder<'a> {
    /// Creates a new task builder associated with the given task manager.
    pub(crate) fn new(spawner: &'a TaskSpawner) -> Self {
        Self {
            spawner,
            name: None,
            instrument: false,
            graceful_shutdown: false,
            _phantom: PhantomData,
        }
    }

    /// Notifies the task manager to perform a graceful shutdown when the task is finished.
    pub fn graceful_shutdown(mut self) -> Self {
        self.graceful_shutdown = true;
        self
    }

    pub fn critical(self) -> TaskBuilder<'a, CriticalTask> {
        // TaskBuilder { builder: self.graceful_shutdown() }
        TaskBuilder {
            name: self.name,
            spawner: self.spawner,
            instrument: self.instrument,
            graceful_shutdown: true,
            _phantom: PhantomData,
        }
    }

    /// Spawns the given future based on the configured builder.
    pub fn spawn<F>(self, fut: F) -> TaskHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.spawner.spawn_on_manager(self.build_future(fut))
    }
}

impl<'a, T> TaskBuilder<'a, T> {
    /// Sets the name of the task.
    pub fn name(mut self, name: &str) -> Self {
        self.name = Some(name.to_string());
        self
    }

    /// Instruments the task for collecting metrics. Is a no-op for now.
    pub fn instrument(mut self) -> Self {
        self.instrument = true;
        self
    }

    fn build_future<F>(&self, fut: F) -> impl Future<Output = F::Output>
    where
        F: Future + Send + 'static,
    {
        let instrument = self.instrument;
        let graceful_shutdown = self.graceful_shutdown;
        let cancellation_token = self.spawner.cancellation_token().clone();
        let task_name = self.name.clone().unwrap_or_else(|| "unnamed".to_string());

        // Creates a future that will send a cancellation signal to the manager when the future is
        // completed, regardless of success or error.
        let fut = {
            fut.map(move |res| {
                if graceful_shutdown {
                    debug!(target: "tasks", task = task_name, "Task with graceful shutdown completed.");
                    cancellation_token.cancel();
                }
                res
            })
        };

        if instrument {
            // TODO: store the TaskMonitor
            let monitor = TaskMonitor::new();
            Either::Left(monitor.instrument(fut))
        } else {
            Either::Right(fut)
        }
    }
}

impl<'a> TaskBuilder<'a, CriticalTask> {
    pub fn spawn<F>(self, fut: F) -> TaskHandle<()>
    where
        F: Future + Send + 'static,
    {
        let cancellation_token = self.spawner.cancellation_token().clone();
        let task_name = self.name.clone().unwrap_or_else(|| "unnamed".to_string());

        let fut = AssertUnwindSafe(fut)
            .catch_unwind()
            .map_err(move |error| {
                let error = PanickedTaskError { error };
                error!(%error, task = task_name, "Critical task failed.");
                cancellation_token.cancel();
                error
            })
            .map(drop);

        self.spawner.spawn_on_manager(self.build_future(fut))
    }
}

/// A simple wrapper type so that we can implement [`std::error::Error`] for `Box<dyn Any + Send>`.
#[derive(Debug, Error)]
pub struct PanickedTaskError {
    /// The error that caused the panic. It is a boxed `dyn Any` due to the future returned by
    /// [`catch_unwind`](futures::future::FutureExt::catch_unwind).
    error: Box<dyn Any + Send>,
}

impl std::fmt::Display for PanickedTaskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.error.downcast_ref::<String>() {
            None => Ok(()),
            Some(msg) => write!(f, "{msg}"),
        }
    }
}
