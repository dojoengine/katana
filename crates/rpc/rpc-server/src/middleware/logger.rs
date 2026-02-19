use std::future::Future;

use jsonrpsee::core::middleware;
use jsonrpsee::core::middleware::{Batch, Notification};
use jsonrpsee::types::Request;
use tracing::Instrument;

/// RPC logger layer.
#[derive(Copy, Clone, Debug)]
pub struct RpcLoggerLayer;

impl RpcLoggerLayer {
    /// Create a new RPC logging layer.
    pub fn new() -> Self {
        Self
    }
}

impl Default for RpcLoggerLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl<S> tower::Layer<S> for RpcLoggerLayer {
    type Service = RpcLogger<S>;

    fn layer(&self, service: S) -> Self::Service {
        RpcLogger { service }
    }
}

/// A middleware that logs each RPC call.
#[derive(Debug, Clone)]
pub struct RpcLogger<S> {
    service: S,
}

impl<S> middleware::RpcServiceT for RpcLogger<S>
where
    S: middleware::RpcServiceT + Send + Sync + Clone + 'static,
{
    type BatchResponse = S::BatchResponse;
    type MethodResponse = S::MethodResponse;
    type NotificationResponse = S::NotificationResponse;

    #[inline]
    #[tracing::instrument(target = "rpc", level = "trace", name = "rpc_call", skip_all, fields(method = req.method_name()))]
    fn call<'a>(&self, req: Request<'a>) -> impl Future<Output = Self::MethodResponse> + Send + 'a {
        self.service.call(req).in_current_span()
    }

    #[inline]
    #[tracing::instrument(target = "rpc", level = "trace", name = "rpc_batch", skip_all, fields(batch_size = batch.len()) )]
    fn batch<'a>(&self, batch: Batch<'a>) -> impl Future<Output = Self::BatchResponse> + Send + 'a {
        self.service.batch(batch).in_current_span()
    }

    #[inline]
    #[tracing::instrument(target = "rpc", level = "trace", name = "rpc_notification", skip_all, fields(method = &*n.method))]
    fn notification<'a>(
        &self,
        n: Notification<'a>,
    ) -> impl Future<Output = Self::NotificationResponse> + Send + 'a {
        self.service.notification(n).in_current_span()
    }
}
