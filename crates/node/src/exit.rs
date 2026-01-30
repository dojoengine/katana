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
pub struct NodeStoppedFuture<'a, P> {
    fut: BoxFuture<'a, Result<()>>,
    _phantom: std::marker::PhantomData<P>,
}

impl<'a, P> NodeStoppedFuture<'a, P>
where
    P: ProviderFactory,
    <P as ProviderFactory>::Provider: ProviderRO,
    <P as ProviderFactory>::ProviderMut: ProviderRW,
{
    pub(crate) fn new(handle: &'a LaunchedNode<P>) -> Self {
        // Clone the handles we need so we can move them into the async block.
        // This avoids capturing `&LaunchedNode<P>` which isn't Sync.

        let rpc = handle.rpc.clone();
        let grpc = handle.grpc.clone();
        let gateway = handle.gateway.clone();
        let task_manager = handle.node.task_manager.clone();

        let fut = Box::pin(async move {
            task_manager.wait_for_shutdown().await;
            rpc.stop()?;

            if let Some(grpc) = grpc {
                grpc.stop()?;
            }

            if let Some(gw) = gateway {
                gw.stop()?;
            }

            Ok(())
        });

        Self { fut, _phantom: std::marker::PhantomData }
    }
}

impl<P> Future for NodeStoppedFuture<'_, P>
where
    P: ProviderFactory + Unpin,
    <P as ProviderFactory>::Provider: ProviderRO,
    <P as ProviderFactory>::ProviderMut: ProviderRW,
{
    type Output = Result<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        this.fut.poll_unpin(cx)
    }
}

impl<P> core::fmt::Debug for NodeStoppedFuture<'_, P>
where
    P: ProviderFactory,
    <P as ProviderFactory>::Provider: ProviderRO,
    <P as ProviderFactory>::ProviderMut: ProviderRW,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NodeStoppedFuture").field("fut", &"...").finish()
    }
}
