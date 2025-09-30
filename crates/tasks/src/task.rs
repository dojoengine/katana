use std::any::Any;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use tracing::error;

pub type Result<T> = std::result::Result<T, JoinError>;

#[derive(Debug)]
pub struct JoinHandle<T>(pub(crate) tokio::task::JoinHandle<Result<T>>);

impl<T> JoinHandle<T> {
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
    type Output = Result<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let inner = unsafe { self.map_unchecked_mut(|s| &mut s.0) };
        match inner.poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(res)) => Poll::Ready(res),
            Poll::Ready(Err(err)) => Poll::Ready(Err(err.into())),
        }
    }
}

/// Task failed to execute to completion.
#[derive(Debug, thiserror::Error)]
pub enum JoinError {
    #[error("task was canceled")]
    Cancelled,
    #[error("task panicked")]
    Panic(Box<dyn std::any::Any + Send>),
}

impl JoinError {
    /// Returns `true` if the error represents a task cancellation.
    pub fn is_cancelled(&self) -> bool {
        matches!(self, JoinError::Cancelled)
    }

    /// Returns `true` if the error represents a task panic.
    pub fn is_panic(&self) -> bool {
        matches!(self, JoinError::Panic(_))
    }

    /// Consumes the join error, returning the object with which the task panicked.
    ///
    /// # Panics
    ///
    /// `into_panic()` panics if the `Error` does not represent the underlying
    /// task terminating with a panic. Use `is_panic` to check the error reason
    /// or `try_into_panic` for a variant that does not panic.
    #[track_caller]
    pub fn into_panic(self) -> Box<dyn Any + Send + 'static> {
        self.try_into_panic().expect("`JoinError` reason is not a panic.")
    }

    /// Consumes the join error, returning the object with which the task
    /// panicked if the task terminated due to a panic. Otherwise, `self` is
    /// returned.
    pub fn try_into_panic(self) -> std::result::Result<Box<dyn Any + Send + 'static>, JoinError> {
        match self {
            Self::Panic(p) => Ok(p),
            _ => Err(self),
        }
    }
}

impl From<tokio::task::JoinError> for JoinError {
    fn from(value: tokio::task::JoinError) -> Self {
        match value.try_into_panic() {
            Ok(panic) => Self::Panic(panic),
            Err(..) => Self::Cancelled,
        }
    }
}
