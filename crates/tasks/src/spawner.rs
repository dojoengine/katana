use core::future::Future;
use core::marker::PhantomData;
use core::pin::Pin;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;

use futures::stream::FuturesUnordered;
use futures::{FutureExt, StreamExt};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;
use tokio::task::block_in_place;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error};

use crate::{CpuBlockingJoinHandle, Inner, JoinError, JoinHandle};

type BoxScopedFuture<'scope> = Pin<Box<dyn Future<Output = ()> + Send + 'scope>>;
type BoxScopeFuture<'scope, T> = Pin<Box<dyn Future<Output = T> + Send + 'scope>>;

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

    /// Runs a scoped block in which tasks may borrow non-`'static` data but are guaranteed to be
    /// completed before this method returns.
    pub async fn scope<'scope, F, R>(
        &'scope self,
        f: impl FnOnce(ScopedTaskSpawner<'scope>) -> F,
    ) -> R
    where
        F: Future<Output = R> + Send + 'scope,
        R: Send + 'scope,
    {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let scoped_spawner: ScopedTaskSpawner<'scope> =
            ScopedTaskSpawner { inner: self.inner.clone(), sender, _marker: PhantomData };

        let user_future = Box::pin(f(scoped_spawner.clone()));

        TaskScope::new(scoped_spawner, receiver, user_future).run().await
    }

    pub(crate) fn cancellation_token(&self) -> &CancellationToken {
        &self.inner.on_cancel
    }
}

/// Spawner used inside [`TaskSpawner::scope`] to spawn scoped tasks.
#[derive(Debug, Clone)]
pub struct ScopedTaskSpawner<'scope> {
    inner: Arc<Inner>,
    sender: UnboundedSender<BoxScopedFuture<'scope>>,
    _marker: PhantomData<&'scope ()>,
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

        let task = Box::pin(async move {
            let result = cancellable(cancellation_token, fut).await;
            let _ = tx.send(result);
        });

        if self.sender.send(task).is_err() {
            receiver.close();
        }

        ScopedJoinHandle { receiver, _marker: PhantomData }
    }

    /// Spawn a blocking task that can borrow data with lifetime `'scope`.
    pub fn spawn_blocking<F, R>(&self, func: F) -> ScopedJoinHandle<'scope, R>
    where
        F: FnOnce() -> R + Send + 'scope,
        R: Send + 'scope,
    {
        let (tx, rx) = oneshot::channel();
        let mut receiver = rx;
        let cancellation_token = self.inner.on_cancel.clone();

        let task = Box::pin(async move {
            let result = block_in_place(|| {
                if cancellation_token.is_cancelled() {
                    return Err(JoinError::Cancelled);
                }

                panic::catch_unwind(AssertUnwindSafe(func)).map_err(JoinError::Panic)
            });

            let _ = tx.send(result);
        });

        if self.sender.send(task).is_err() {
            receiver.close();
        }

        ScopedJoinHandle { receiver, _marker: PhantomData }
    }
}

struct TaskScope<'scope, R>
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
    fn new(
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

    async fn run(mut self) -> R {
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

/// Join handle for scoped tasks spawned via [`TaskSpawner::scope`].
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

    fn poll(
        self: Pin<&mut Self>,
        cx: &mut core::task::Context<'_>,
    ) -> core::task::Poll<Self::Output> {
        let this = self.get_mut();
        match Pin::new(&mut this.receiver).poll(cx) {
            core::task::Poll::Ready(Ok(value)) => core::task::Poll::Ready(value),
            core::task::Poll::Ready(Err(..)) => core::task::Poll::Ready(Err(JoinError::Cancelled)),
            core::task::Poll::Pending => core::task::Poll::Pending,
        }
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
    pub fn spawn<F, R>(&self, func: F) -> CpuBlockingJoinHandle<R>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        // TODO(kariy): use TaskBuilder::build_blocking_task to buil the task
        self.0.inner.blocking_pool.spawn(func)
    }
}

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
    /// completion.
    graceful_shutdown: bool,
}

impl<'a> TaskBuilder<'a> {
    /// Creates a new task builder associated with the given task manager.
    pub(crate) fn new(spawner: &'a TaskSpawner) -> Self {
        Self { spawner, name: None, graceful_shutdown: false }
    }

    /// Sets the name of the task.
    pub fn name<T: Into<String>>(mut self, name: T) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Upon the task completion (either successfully or due to a panic), notify the task manager to
    /// perform a graceful shutdown.
    pub fn graceful_shutdown(mut self) -> TaskBuilder<'a> {
        self.graceful_shutdown = true;
        self
    }

    /// Spawns the given future based on the configured builder.
    pub fn spawn<F>(self, fut: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.spawner.spawn(self.build_task(fut))
    }

    /// Spawns an blocking task onto the Tokio runtime associated with the manager.
    ///
    /// # CPU-bound tasks
    ///
    /// If need to spawn a CPU-bound blocking task, consider using the
    /// [`CPUBoundTaskSpawner::spawn`] instead.
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

        // Tokio already catches panics in the spawned task, but we are unable to handle it directly
        // inside the task without awaiting on it. So we catch it ourselves and resume it
        // again for tokio to catch it.
        AssertUnwindSafe(fut).catch_unwind().map(move |result| match result {
            Ok(value) => {
	            if graceful_shutdown {
	                debug!(target: "tasks", task = ?task_name, "Task with graceful shutdown completed.");
	                cancellation_token.cancel();
	            }
	           value
            },
            Err(error) => {
	            // get the panic reason message
                let reason = error.downcast_ref::<String>();
                error!(target = "tasks", task = ?task_name, ?reason, "Task panicked.");

                if graceful_shutdown {
                    cancellation_token.cancel();
                }

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
            match panic::catch_unwind(AssertUnwindSafe(func)) {
                Ok(value) => {
                    if graceful_shutdown {
                        debug!(target: "tasks", task = ?task_name, "Task with graceful shutdown completed.");
                        cancellation_token.cancel();
                    }
                    value
                }
                Err(error) => {
                    // get the panic reason message
                    let reason = error.downcast_ref::<String>();
                    error!(target = "tasks", task = ?task_name, ?reason, "Task panicked.");

                    if graceful_shutdown {
                        cancellation_token.cancel();
                    }

                    std::panic::resume_unwind(error);
                }
            }
        }
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
    use std::sync::atomic::{AtomicUsize, Ordering};

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

    #[tokio::test]
    async fn scoped_spawn_allows_borrowed_data() {
        let manager = TaskManager::current();
        let spawner = manager.task_spawner();

        let text = String::from("scoped");
        let expected_len = text.len();

        spawner
            .scope(|scope| {
                let text_ref: &String = &text;

                async move {
                    let handle = scope.spawn(async move { text_ref.len() });
                    assert_eq!(handle.await.unwrap(), expected_len);
                }
            })
            .await;

        // original value is still valid after scope returns
        assert_eq!(text.len(), expected_len);
    }

    #[tokio::test]
    async fn scoped_spawn_blocking_allows_borrowed_data() {
        let manager = TaskManager::current();
        let spawner = manager.task_spawner();

        let counter = AtomicUsize::new(0);

        spawner
            .scope(|scope| {
                let counter_ref = &counter;

                async move {
                    let handle = scope.spawn_blocking(move || {
                        counter_ref.fetch_add(1, Ordering::SeqCst);
                        counter_ref.load(Ordering::SeqCst)
                    });

                    assert_eq!(handle.await.unwrap(), 1);
                }
            })
            .await;

        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }
}
