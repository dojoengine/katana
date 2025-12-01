use core::future::Future;
use core::marker::PhantomData;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;

use futures::FutureExt;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error};

use crate::scope::{ScopedTaskSpawner, TaskScope};
use crate::{Cancellable, CpuBlockingJoinHandle, JoinHandle, TaskManagerInner};

/// A spawner for spawning tasks on the [`TaskManager`] that it was derived from.
///
/// This is the main way to spawn tasks on a [`TaskManager`]. It can only be created
/// by calling [`TaskManager::task_spawner`].
#[derive(Debug, Clone)]
pub struct TaskSpawner {
    /// A handle to the [`TaskManager`] that this spawner is associated with.
    pub(crate) inner: Arc<TaskManagerInner>,
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
        let task = Cancellable::new(cancel_token, fut);

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
    ///
    /// Unlike [`TaskSpawner::spawn`], scoped tasks don't need to be `'static`. Without a scope, the
    /// compiler prevents you from capturing stack data because the task could outlive the
    /// borrow:
    ///
    /// ```compile_fail
    /// # use katana_tasks::TaskManager;
    /// # #[tokio::main] async fn main() {
    /// let manager = TaskManager::current();
    /// let spawner = manager.task_spawner();
    /// let prefix = String::from("Hello world!");
    ///
    /// // ERROR: `spawn` requires futures to be `'static`.
    /// spawner.spawn(async { println!("{prefix}") });
    /// # }
    /// ```
    ///
    /// A scope keeps everything within the lifetime of the call, so you can borrow safely.
    ///
    /// # Examples
    ///
    /// Borrow stack data in an async task:
    /// ```
    /// # use katana_tasks::TaskManager;
    /// # #[tokio::main]
    /// async fn main() {
    /// let manager = TaskManager::current();
    /// let spawner = manager.task_spawner();
    ///
    /// let prefix = String::from("hi ");
    /// let msg = spawner.scope(|scope| async move {
    ///     scope.spawn(async move { format!("{prefix}there") }).await.unwrap()
    /// }).await;
    ///
    /// assert_eq!(msg, "hi there");
    /// # }
    /// ```
    ///
    /// Spawn multiple scoped tasks and join them:
    /// ```
    /// # use katana_tasks::TaskManager;
    /// # #[tokio::main]
    /// async fn main() {
    /// let manager = TaskManager::current();
    /// let spawner = manager.task_spawner();
    ///
    /// let numbers = vec![1, 2, 3];
    /// let sum = spawner.scope(|scope| {
    ///     let numbers = numbers.clone();
    ///     async move {
    ///         let handles: Vec<_> = numbers
    ///             .into_iter()
    ///             .map(|n| scope.spawn(async move { n * 2 }))
    ///             .collect();
    ///
    ///         let mut total = 0;
    ///         for h in handles {
    ///             total += h.await.unwrap();
    ///         }
    ///         total
    ///     }
    /// }).await;
    ///
    /// assert_eq!(sum, 12);
    /// # }
    /// ```
    ///
    /// Use scoped blocking work with borrowed data (requires a multithreaded Tokio runtime):
    /// ```
    /// # use katana_tasks::TaskManager;
    /// # use std::sync::{Arc, Mutex};
    /// # #[tokio::main] async fn main() {
    /// let manager = TaskManager::current();
    /// let spawner = manager.task_spawner();
    ///
    /// let items = Arc::new(Mutex::new(vec![1, 2]));
    /// let handle_items = items.clone();
    ///
    /// spawner
    ///     .scope(|scope| {
    ///         let handle_items = handle_items.clone();
    ///         async move {
    ///             scope
    ///                 .spawn_blocking(move || {
    ///                     let mut guard = handle_items.lock().unwrap();
    ///                     guard.push(3);
    ///                 })
    ///                 .await
    ///                 .unwrap();
    ///         }
    ///     })
    ///     .await;
    ///
    /// assert_eq!(*items.lock().unwrap(), vec![1, 2, 3]);
    /// # }
    /// ```
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

        let scoped = TaskScope::new(scoped_spawner, receiver, user_future).run();
        // Track the scoped execution so it participates in shutdown/wait semantics.
        self.inner.tracker.track_future(scoped).await
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

#[cfg(test)]
mod tests {

    use std::future::pending;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tokio::sync::oneshot;
    use tokio::task::yield_now;

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
    async fn scoped_tasks_are_tracked() {
        let manager = TaskManager::current();
        let spawner = manager.task_spawner();

        assert_eq!(manager.tracked_tasks(), 0);

        // channel for manual trigger on the scoped task's completion
        let (tx, rx) = oneshot::channel::<()>();
        let spawner_clone = spawner.clone();

        let scoped = tokio::spawn(async move {
            let _ = spawner_clone
                .scope(|scope| {
                    scope.spawn(async {
                        let _ = rx.await;
                    })
                })
                .await;
        });

        yield_now().await; // allow the scoped task and its inner task to register with the tracker

        // 1 for the inner task (scope.spawn), 1 for the scoped task (spawner_clone.scope)
        assert_eq!(manager.tracked_tasks(), 2);

        let _ = tx.send(()); // signal the scoped task to complete
        scoped.await.unwrap();

        assert_eq!(manager.tracked_tasks(), 0);
    }

    #[tokio::test]
    async fn scoped_spawn_allows_borrowed_data() {
        let manager = TaskManager::current();
        let spawner = manager.task_spawner();

        let text = String::from("scoped");

        let returned_ref = spawner
            .scope(|scope| {
                let text_ref: &String = &text;
                scope.spawn(async move { text_ref })
            })
            .await
            .unwrap();

        // original value is still valid after scope returns
        assert_eq!(text, *returned_ref);
    }

    #[tokio::test(flavor = "multi_thread")]
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

        let mut prefix = String::from("hello ");
        spawner
            .scope(|scope| {
                scope.spawn(async {
                    prefix.push_str("world!");
                })
            })
            .await
            .unwrap();

        assert_eq!(prefix.as_str(), "hello world!")
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn scoped_cpu_bound_allows_borrowed_data() {
        let manager = TaskManager::current();
        let spawner = manager.task_spawner();

        let counter = AtomicUsize::new(0);

        spawner
            .scope(|scope| {
                let cpu = scope.cpu_bound();
                let counter_ref = &counter;

                async move {
                    let handle = cpu.spawn(move || {
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
