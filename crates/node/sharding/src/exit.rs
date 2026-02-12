use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use anyhow::Result;
use futures::future::BoxFuture;
use futures::FutureExt;

use super::LaunchedShardNode;

/// A Future that is resolved once the shard node has been stopped including all running tasks.
#[must_use = "futures do nothing unless polled"]
pub struct ShardNodeStoppedFuture<'a> {
    fut: BoxFuture<'a, Result<()>>,
}

impl<'a> ShardNodeStoppedFuture<'a> {
    pub(crate) fn new(handle: &'a LaunchedShardNode) -> Self {
        let rpc = handle.rpc.clone();
        let task_manager = handle.node.task_manager.clone();
        let runtime = handle.node.runtime.lock().take();
        let task_spawner = task_manager.task_spawner();

        let fut = Box::pin(async move {
            task_manager.wait_for_shutdown().await;

            if let Some(runtime) = runtime {
                let _ = task_spawner
                    .spawn_blocking(move || {
                        runtime.shutdown_timeout(Duration::from_secs(30));
                    })
                    .await;
            }

            rpc.stop()?;
            Ok(())
        });

        Self { fut }
    }
}

impl Future for ShardNodeStoppedFuture<'_> {
    type Output = Result<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        this.fut.poll_unpin(cx)
    }
}

impl core::fmt::Debug for ShardNodeStoppedFuture<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ShardNodeStoppedFuture").finish_non_exhaustive()
    }
}
