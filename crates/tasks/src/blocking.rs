use std::any::Any;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::channel::oneshot;
use rayon::ThreadPoolBuilder;

use crate::task::JoinError;

/// This `struct` is created by the [`BlockingTaskPoolBuilder::build`] method.
#[derive(Debug, thiserror::Error)]
#[error("failed to initialize blocking thread pool: {0}")]
pub struct CpuBlockingTaskPoolInitError(rayon::ThreadPoolBuildError);

/// A builder for configuring a [BlockingTaskPool].
#[derive(Debug)]
pub struct CpuBlockingTaskPoolBuilder {
    inner: ThreadPoolBuilder,
}

impl CpuBlockingTaskPoolBuilder {
    /// Creates a new builder with default configuration.
    pub fn new() -> Self {
        Self { inner: ThreadPoolBuilder::new() }
    }

    /// Sets the name for the spawned worker threads.
    ///
    /// The closure takes a thread index and returns the thread's name.
    pub fn thread_name<F>(mut self, closure: F) -> Self
    where
        F: FnMut(usize) -> String + 'static,
    {
        self.inner = self.inner.thread_name(closure);
        self
    }

    /// Builds the [`CpuBlockingTaskPool`] with the configured settings.
    pub fn build(self) -> Result<CpuBlockingTaskPool, CpuBlockingTaskPoolInitError> {
        self.inner
            .build()
            .map(|pool| CpuBlockingTaskPool { pool: Arc::new(pool) })
            .map_err(CpuBlockingTaskPoolInitError)
    }
}

impl Default for CpuBlockingTaskPoolBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// A thread-pool for spawning blocking tasks.
///
/// This is a simple wrapper around *rayon*'s thread-pool. This is mainly for executing expensive
/// CPU-bound tasks.
#[derive(Debug, Clone)]
pub struct CpuBlockingTaskPool {
    pool: Arc<rayon::ThreadPool>,
}

impl CpuBlockingTaskPool {
    /// Creates a new [`CpuBlockingTaskPool`] with the given *rayon* thread pool.
    pub fn new(rayon_pool: rayon::ThreadPool) -> Self {
        Self { pool: Arc::new(rayon_pool) }
    }

    /// Creates a new [`CpuBlockingTaskPool`] with the default configuration.
    pub fn builder() -> CpuBlockingTaskPoolBuilder {
        CpuBlockingTaskPoolBuilder::new()
    }

    /// Spawns an asynchronous task in this thread pool, returning a handle for waiting on the
    /// result asynchronously.
    pub fn spawn<F, R>(&self, func: F) -> CpuBlockingJoinHandle<R>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        self.pool.spawn(move || {
            let _ = tx.send(std::panic::catch_unwind(AssertUnwindSafe(func)));
        });
        CpuBlockingJoinHandle { inner: rx }
    }

    pub(crate) fn scope<'scope, F, R>(&self, func: F) -> R
    where
        F: for<'s> FnOnce(&'s rayon::Scope<'scope>) -> R + Send + 'scope,
        R: Send + 'scope,
    {
        self.pool.scope(func)
    }
}

#[derive(Debug)]
pub struct CpuBlockingJoinHandle<T> {
    pub(crate) inner: oneshot::Receiver<std::result::Result<T, Box<dyn Any + Send>>>,
}

impl<T> Future for CpuBlockingJoinHandle<T> {
    type Output = crate::task::Result<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut self.get_mut().inner).poll(cx) {
            Poll::Ready(Ok(result)) => match result {
                Ok(value) => Poll::Ready(Ok(value)),
                Err(panic) => Poll::Ready(Err(JoinError::Panic(panic))),
            },
            Poll::Ready(Err(..)) => Poll::Ready(Err(JoinError::Cancelled)),
            Poll::Pending => Poll::Pending,
        }
    }
}
