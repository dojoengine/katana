use core::future::Future;
use core::marker::PhantomData;
use core::pin::Pin;
use std::panic::{self, AssertUnwindSafe};
use std::sync::{mpsc, Arc};
use std::task::Poll;

use futures::stream::FuturesUnordered;
use futures::StreamExt;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;
use tokio::task::block_in_place;

use crate::{Cancellable, JoinError, TaskManagerInner};

type BoxScopedFuture<'scope> = Pin<Box<dyn Future<Output = ()> + Send + 'scope>>;
type BoxScopeFuture<'scope, T> = Pin<Box<dyn Future<Output = T> + Send + 'scope>>;

/// Spawner used inside [`TaskSpawner::scope`] to spawn scoped tasks.
#[derive(Debug, Clone)]
pub struct ScopedTaskSpawner<'scope> {
    pub(crate) inner: Arc<TaskManagerInner>,
    pub(crate) sender: UnboundedSender<BoxScopedFuture<'scope>>,
    pub(crate) _marker: PhantomData<&'scope ()>,
}

impl<'scope> ScopedTaskSpawner<'scope> {
    /// Spawn an async task that can borrow data with lifetime `'scope`.
    pub fn spawn<F>(&self, fut: F) -> ScopedJoinHandle<'scope, F::Output>
    where
        F: Future + Send + 'scope,
        F::Output: Send + 'scope,
    {
        let (tx, rx) = oneshot::channel();
        let mut receiver = rx;
        let cancellation_token = self.inner.on_cancel.clone();

        let task = async move {
            let result = Cancellable::new(cancellation_token, fut).await;
            let _ = tx.send(result);
        };

        let task = self.inner.tracker.track_future(task);
        let task = Box::pin(task);

        if self.sender.send(task).is_err() {
            receiver.close();
        }

        ScopedJoinHandle { receiver, _marker: PhantomData }
    }

    /// Returns a scoped spawner for CPU-bound work.
    pub fn cpu_bound(&self) -> ScopedCpuTaskSpawner<'scope> {
        ScopedCpuTaskSpawner {
            inner: self.inner.clone(),
            sender: self.sender.clone(),
            _marker: PhantomData,
        }
    }

    /// Spawn a blocking task that can borrow data with lifetime `'scope`.
    ///
    /// # Runtime requirement
    ///
    /// This uses [`tokio::task::block_in_place`] under the hood, which only works on
    /// Tokio's multi-threaded runtime. On a current-thread runtime this will panic. If
    /// you need to support a current-thread runtime, prefer:
    ///
    /// 1. Moving/cloning the data so the closure can be `'static` and using the non-scoped
    ///    `TaskSpawner::spawn_blocking`, or
    /// 2. Running the work inline (accepting that it will block the current thread).
    ///
    /// The scoped variant exists to let you borrow non-`'static` data; that capability
    /// currently depends on `block_in_place`.
    pub fn spawn_blocking<F, R>(&self, func: F) -> ScopedJoinHandle<'scope, R>
    where
        F: FnOnce() -> R + Send + 'scope,
        R: Send + 'scope,
    {
        let (tx, rx) = oneshot::channel();
        let mut receiver = rx;
        let cancellation_token = self.inner.on_cancel.clone();

        let task = async move {
            let result = block_in_place(|| {
                if cancellation_token.is_cancelled() {
                    return Err(JoinError::Cancelled);
                }

                panic::catch_unwind(AssertUnwindSafe(func)).map_err(JoinError::Panic)
            });

            let _ = tx.send(result);
        };

        let task = self.inner.tracker.track_future(task);
        let task = Box::pin(task);

        if self.sender.send(task).is_err() {
            receiver.close();
        }

        ScopedJoinHandle { receiver, _marker: PhantomData }
    }
}

/// Spawner for CPU-bound scoped tasks executed on the dedicated blocking pool.
#[derive(Debug, Clone)]
pub struct ScopedCpuTaskSpawner<'scope> {
    pub(crate) inner: Arc<TaskManagerInner>,
    pub(crate) sender: UnboundedSender<BoxScopedFuture<'scope>>,
    pub(crate) _marker: PhantomData<&'scope ()>,
}

impl<'scope> ScopedCpuTaskSpawner<'scope> {
    /// Spawn a CPU-bound task that can borrow data with lifetime `'scope`.
    ///
    /// # Runtime requirement
    ///
    /// Uses [`tokio::task::block_in_place`] to enter the blocking pool. This only works on
    /// Tokio's multithreaded runtime. On a current-thread runtime this will panic. If you
    /// must support a current-thread runtime, either move/clone data into a `'static` closure
    /// and use the non-scoped `CPUBoundTaskSpawner`, or run the work inline knowing it will
    /// block the thread.
    pub fn spawn<F, R>(&self, func: F) -> ScopedJoinHandle<'scope, R>
    where
        F: FnOnce() -> R + Send + 'scope,
        R: Send + 'scope,
    {
        let (tx, rx) = oneshot::channel();
        let mut receiver = rx;
        let inner = self.inner.clone();

        let task = async move {
            let result = block_in_place(|| {
                if inner.on_cancel.is_cancelled() {
                    return Err(JoinError::Cancelled);
                }

                let (res_tx, res_rx) = mpsc::channel();
                inner.blocking_pool.scope(|scope| {
                    scope.spawn(|_| {
                        let _ = res_tx.send(panic::catch_unwind(AssertUnwindSafe(func)));
                    });
                });

                match res_rx.recv() {
                    Ok(Ok(value)) => Ok(value),
                    Ok(Err(panic)) => Err(JoinError::Panic(panic)),
                    Err(..) => Err(JoinError::Cancelled),
                }
            });

            let _ = tx.send(result);
        };

        let task = self.inner.tracker.track_future(task);
        let task = Box::pin(task);

        if self.sender.send(task).is_err() {
            receiver.close();
        }

        ScopedJoinHandle { receiver, _marker: PhantomData }
    }
}

pub(crate) struct TaskScope<'scope, R>
where
    R: Send + 'scope,
{
    tasks: FuturesUnordered<BoxScopedFuture<'scope>>,
    receiver: UnboundedReceiver<BoxScopedFuture<'scope>>,
    user_future: BoxScopeFuture<'scope, R>,
    spawner: Option<ScopedTaskSpawner<'scope>>,
    receiver_closed: bool,
}

impl<'scope, R> TaskScope<'scope, R>
where
    R: Send + 'scope,
{
    pub(crate) fn new(
        spawner: ScopedTaskSpawner<'scope>,
        receiver: UnboundedReceiver<BoxScopedFuture<'scope>>,
        user_future: BoxScopeFuture<'scope, R>,
    ) -> Self {
        Self {
            tasks: FuturesUnordered::new(),
            receiver,
            user_future,
            spawner: Some(spawner),
            receiver_closed: false,
        }
    }

    pub(crate) async fn run(mut self) -> R {
        let mut user_output = None;
        let mut user_done = false;

        loop {
            tokio::select! {
                result = self.receiver.recv(), if !self.receiver_closed => {
                    match result {
                        Some(task) => self.tasks.push(task),
                        None => self.receiver_closed = true,
                    }
                }

                Some(_) = self.tasks.next(), if !self.tasks.is_empty() => {}

                output = self.user_future.as_mut(), if !user_done => {
                    user_output = Some(output);
                    user_done = true;
                    // Drop the spawner to close the sender side of the channel.
                    self.spawner.take();
                }

                else => {
                    if user_output.is_some() && self.receiver_closed && self.tasks.is_empty() {
                        break;
                    }
                }
            }
        }

        user_output.expect("user future completed")
    }
}

/// Join handle for scoped tasks spawned via
/// [`TaskSpawner::scope`](crate::spawner::TaskSpawner::scope).
pub struct ScopedJoinHandle<'scope, T> {
    receiver: oneshot::Receiver<crate::Result<T>>,
    _marker: PhantomData<&'scope ()>,
}

impl<T> core::fmt::Debug for ScopedJoinHandle<'_, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ScopedJoinHandle").finish()
    }
}

impl<T> Future for ScopedJoinHandle<'_, T> {
    type Output = crate::Result<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut core::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        match Pin::new(&mut this.receiver).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(value)) => Poll::Ready(value),
            Poll::Ready(Err(..)) => Poll::Ready(Err(JoinError::Cancelled)),
        }
    }
}
