use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use anyhow::Result;
use futures::future::BoxFuture;
use futures::FutureExt;
use katana_provider::{ProviderFactory, ProviderRO, ProviderRW};

use crate::LaunchedNode;

/// A Future that is resolved once the node has been stopped including all of its running tasks.
#[must_use = "futures do nothing unless polled"]
pub struct NodeStoppedFuture<'a> {
    fut: BoxFuture<'a, Result<()>>,
}

impl<'a> NodeStoppedFuture<'a> {
    pub(crate) fn new<P>(handle: &'a LaunchedNode<P>) -> Self
    where
        P: ProviderFactory + Clone,
        <P as ProviderFactory>::Provider: ProviderRO,
        <P as ProviderFactory>::ProviderMut: ProviderRW,
    {
        // Clone the handles we need so we can move them into the async block.
        // This avoids capturing `&LaunchedNode<P>` which isn't Sync.

        let rpc = handle.rpc.clone();
        #[cfg(feature = "grpc")]
        let grpc = handle.grpc.clone();
        let gateway = handle.gateway.clone();
        let task_manager = handle.node.task_manager.clone();

        let fut = Box::pin(async move {
            task_manager.wait_for_shutdown().await;
            rpc.stop()?;

            #[cfg(feature = "grpc")]
            if let Some(grpc) = grpc {
                grpc.stop()?;
            }

            if let Some(gw) = gateway {
                gw.stop()?;
            }

            Ok(())
        });

        Self { fut }
    }
}

impl Future for NodeStoppedFuture<'_> {
    type Output = Result<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        this.fut.poll_unpin(cx)
    }
}

impl core::fmt::Debug for NodeStoppedFuture<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NodeStoppedFuture").finish_non_exhaustive()
    }
}
