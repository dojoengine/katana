//! RPC implementations.

#![allow(clippy::blocks_in_conditions)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use jsonrpsee::core::middleware::RpcServiceBuilder;
use jsonrpsee::core::{RegisterMethodError, TEN_MB_SIZE_BYTES};
use jsonrpsee::server::{ServerConfig, ServerHandle, StopHandle, TowerServiceBuilder};
use jsonrpsee::{Methods, RpcModule};
use tracing::info;

#[cfg(feature = "cartridge")]
pub mod cartridge;
#[cfg(feature = "paymaster")]
pub mod paymaster;

#[cfg(feature = "tee")]
pub mod tee;

pub mod cors;
pub mod dev;
pub mod health;
pub mod metrics;
pub mod permit;
pub mod starknet;
pub mod txpool;

mod logger;
mod utils;
use cors::Cors;
use health::HealthCheck;
pub use jsonrpsee::http_client::HttpClient;
pub use katana_rpc_api as api;
use metrics::RpcServerMetricsLayer;

/// The default maximum number of concurrent RPC connections.
pub const DEFAULT_RPC_MAX_CONNECTIONS: u32 = 100;
/// The default maximum number of concurrent estimate_fee requests.
pub const DEFAULT_ESTIMATE_FEE_MAX_CONCURRENT_REQUESTS: u32 = 10;
/// The default maximum size in bytes for an RPC request body.
pub const DEFAULT_MAX_REQUEST_BODY_SIZE: u32 = TEN_MB_SIZE_BYTES;
/// The default maximum size in bytes for an RPC response body.
pub const DEFAULT_MAX_RESPONSE_BODY_SIZE: u32 = TEN_MB_SIZE_BYTES;
/// The default timeout for an RPC request.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    RegisterMethod(#[from] RegisterMethodError),

    #[error("RPC server has already been stopped")]
    AlreadyStopped,

    #[error(transparent)]
    Client(#[from] jsonrpsee::core::ClientError),
}

/// The RPC server handle.
#[derive(Debug, Clone)]
pub struct RpcServerHandle {
    /// The actual address that the server is binded to.
    addr: SocketAddr,
    /// The handle to the spawned [`jsonrpsee::server::Server`].
    handle: ServerHandle,
}

impl RpcServerHandle {
    /// Tell the server to stop without waiting for the server to stop.
    pub fn stop(&self) -> Result<(), Error> {
        self.handle.stop().map_err(|_| Error::AlreadyStopped)
    }

    /// Wait until the server has stopped.
    pub async fn stopped(self) {
        self.handle.stopped().await
    }

    /// Returns the socket address the server is listening on.
    pub fn addr(&self) -> &SocketAddr {
        &self.addr
    }

    /// Returns a HTTP client associated with the server.
    pub fn http_client(&self) -> Result<HttpClient, Error> {
        use jsonrpsee::http_client::HttpClientBuilder;
        let url = format!("http://{}", self.addr);
        Ok(HttpClientBuilder::default().build(url)?)
    }
}

/// Builder for the RPC server.
///
/// Modules can be mounted at specific URL path prefixes via [`module_at`](Self::module_at),
/// or merged into the default module (served at `/`) via [`module`](Self::module).
/// The server routes each request by matching the URL path against registered
/// prefixes (longest match wins), falling back to the default module.
///
/// ```rust,ignore
/// let server = RpcServer::new()
///     .module(shared_module)?          // served at `/` (default)
///     .module_at("/rpc/v0_9", v09)?    // served at `/rpc/v0_9`
///     .module_at("/rpc/v0_10", v010)?; // served at `/rpc/v0_10`
/// ```
#[derive(Debug)]
pub struct RpcServer {
    metrics: bool,
    cors: Option<Cors>,
    health_check: bool,
    explorer: bool,

    /// The default module — serves requests that don't match any path-mounted module.
    default_module: RpcModule<()>,
    /// Modules mounted at specific URL path prefixes.
    routes: Vec<(String, RpcModule<()>)>,

    max_connections: u32,
    max_request_body_size: u32,
    max_response_body_size: u32,
    timeout: Duration,
}

impl RpcServer {
    pub fn new() -> Self {
        Self {
            cors: None,
            metrics: false,
            explorer: false,
            health_check: false,
            default_module: RpcModule::new(()),
            routes: Vec::new(),
            max_connections: 100,
            max_request_body_size: TEN_MB_SIZE_BYTES,
            max_response_body_size: TEN_MB_SIZE_BYTES,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// Set the maximum number of connections allowed. Default is 100.
    pub fn max_connections(mut self, max: u32) -> Self {
        self.max_connections = max;
        self
    }

    /// Set the maximum size of a request body (in bytes). Default is 10 MiB.
    pub fn max_request_body_size(mut self, max: u32) -> Self {
        self.max_request_body_size = max;
        self
    }

    /// Set the maximum size of a response body (in bytes). Default is 10 MiB.
    pub fn max_response_body_size(mut self, max: u32) -> Self {
        self.max_response_body_size = max;
        self
    }

    /// Set the timeout for the server. Default is 20 seconds.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Collect metrics about the RPC server.
    ///
    /// See top level module of [`crate::metrics`] to see what metrics are collected.
    pub fn metrics(mut self, enable: bool) -> Self {
        self.metrics = enable;
        self
    }

    /// Enables health checking endpoint via HTTP `GET /health`
    pub fn health_check(mut self, enable: bool) -> Self {
        self.health_check = enable;
        self
    }

    /// Enables explorer.
    pub fn explorer(mut self, enable: bool) -> Self {
        self.explorer = enable;
        self
    }

    pub fn cors(mut self, cors: Cors) -> Self {
        self.cors = Some(cors);
        self
    }

    /// Merges a module into the default module set (served at `/`).
    ///
    /// Methods registered this way are available on all unmatched paths.
    pub fn module(mut self, module: RpcModule<()>) -> Result<Self, Error> {
        self.default_module.merge(module)?;
        Ok(self)
    }

    /// Mounts a module at a specific URL path prefix.
    ///
    /// Requests whose path starts with `prefix` will be dispatched to this
    /// module's methods instead of the default module's.
    ///
    /// ```rust,ignore
    /// server
    ///     .module_at("/rpc/v0_9", v09_module)?
    ///     .module_at("/rpc/v0_10", v010_module)?;
    /// ```
    pub fn module_at(
        mut self,
        prefix: impl Into<String>,
        module: RpcModule<()>,
    ) -> Result<Self, Error> {
        self.routes.push((prefix.into(), module));
        Ok(self)
    }

    pub async fn start(&self, addr: SocketAddr) -> Result<RpcServerHandle, Error> {
        use futures::FutureExt;
        use jsonrpsee::server::{serve_with_graceful_shutdown, stop_channel};
        use katana_tracing::gcloud::GoogleStackDriverMakeSpan;
        use tokio::net::TcpListener;
        use tower::ServiceBuilder;
        use tower_http::trace::TraceLayer;

        // Clone and merge health check into all modules
        let mut default_module = self.default_module.clone();
        let mut routes: Vec<(String, RpcModule<()>)> = self.routes.clone();

        if self.health_check {
            default_module.merge(HealthCheck)?;
            for (_, module) in &mut routes {
                let _ = module.merge(HealthCheck);
            }
        }

        // Build RPC middleware — must happen before converting to Methods.
        let rpc_metrics = self.metrics.then(|| RpcServerMetricsLayer::new(&default_module));

        // Convert to Methods
        let default_methods: Methods = default_module.into();
        let route_methods: Vec<(String, Methods)> =
            routes.into_iter().map(|(path, module)| (path, module.into())).collect();

        // Build HTTP middleware
        let http_tracer = TraceLayer::new_for_http().make_span_with(GoogleStackDriverMakeSpan);
        let health_check_proxy = self.health_check.then(|| HealthCheck::proxy());

        let http_middleware = ServiceBuilder::new()
            .layer(http_tracer)
            .option_layer(self.cors.clone())
            .option_layer(health_check_proxy)
            .timeout(self.timeout);

        #[cfg(feature = "explorer")]
        let http_middleware = {
            let explorer_layer = if self.explorer {
                Some(katana_explorer::ExplorerLayer::builder().embedded().build().unwrap())
            } else {
                None
            };
            http_middleware.option_layer(explorer_layer)
        };

        // Server config
        let cfg = ServerConfig::builder()
            .max_connections(self.max_connections)
            .max_request_body_size(self.max_request_body_size)
            .max_response_body_size(self.max_response_body_size)
            .build();

        // Create the service builder with HTTP middleware baked in.
        let svc_builder = jsonrpsee::server::Server::builder()
            .set_http_middleware(http_middleware)
            .set_config(cfg)
            .to_service_builder();

        let listener = TcpListener::bind(addr).await?;
        let actual_addr = listener.local_addr()?;
        let (stop_hdl, server_handle) = stop_channel();

        // Per-connection state — cloned for every accepted connection.
        #[derive(Clone)]
        struct PerConnection<RpcMiddleware, HttpMiddleware> {
            default_methods: Arc<Methods>,
            route_methods: Arc<Vec<(String, Methods)>>,
            stop_handle: StopHandle,
            svc_builder: TowerServiceBuilder<RpcMiddleware, HttpMiddleware>,
            rpc_metrics: Option<RpcServerMetricsLayer>,
        }

        let per_conn = PerConnection {
            default_methods: Arc::new(default_methods),
            route_methods: Arc::new(route_methods),
            stop_handle: stop_hdl.clone(),
            svc_builder,
            rpc_metrics,
        };

        tokio::spawn(async move {
            loop {
                let stream = tokio::select! {
                    res = listener.accept() => {
                        match res {
                            Ok((stream, _)) => stream,
                            Err(e) => {
                                tracing::error!(target: "rpc", "failed to accept connection: {e:?}");
                                continue;
                            }
                        }
                    }
                    _ = per_conn.stop_handle.clone().shutdown() => break,
                };

                let per_conn = per_conn.clone();
                let stop_handle = per_conn.stop_handle.clone();

                let svc = tower::service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
                    let PerConnection {
                        default_methods,
                        route_methods,
                        stop_handle,
                        svc_builder,
                        rpc_metrics,
                    } = per_conn.clone();

                    // Route by path prefix — first match wins.
                    let path = req.uri().path();
                    let methods = route_methods
                        .iter()
                        .find(|(prefix, _)| path.starts_with(prefix.as_str()))
                        .map(|(_, m)| m.clone())
                        .unwrap_or_else(|| (*default_methods).clone());

                    let rpc_middleware = RpcServiceBuilder::new()
                        .option_layer(rpc_metrics)
                        .layer(logger::RpcLoggerLayer::new());

                    let mut svc =
                        svc_builder.set_rpc_middleware(rpc_middleware).build(methods, stop_handle);

                    async move { tower::Service::call(&mut svc, req).await }.boxed()
                });

                tokio::spawn(serve_with_graceful_shutdown(
                    stream,
                    svc,
                    stop_handle.shutdown(),
                ));
            }
        });

        let handle = RpcServerHandle { handle: server_handle, addr: actual_addr };

        info!(target: "rpc", addr = %handle.addr, "RPC server started.");

        for (path, _) in self.routes.iter() {
            info!(target: "rpc", path = %path, "RPC module mounted at path.");
        }

        if self.explorer {
            let addr = format!("{}/explorer", handle.addr);
            info!(target: "explorer", %addr, "Explorer started.");
        }

        Ok(handle)
    }
}

impl Default for RpcServer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::future::pending;
    use std::time::Duration;

    use jsonrpsee::{rpc_params, RpcModule};

    use crate::RpcServer;

    #[tokio::test]
    async fn test_rpc_server_timeout() {
        use jsonrpsee::core::client::ClientT;

        let mut module = RpcModule::new(());
        module.register_async_method("test_timeout", |_, _, _| pending::<()>()).unwrap();

        let server = RpcServer::new().timeout(Duration::from_millis(200)).module(module).unwrap();

        let addr = "127.0.0.1:0".parse().unwrap();
        let server_handle = server.start(addr).await.unwrap();

        let client = server_handle.http_client().unwrap();
        let result = client.request::<String, _>("test_timeout", rpc_params![]).await;

        assert!(result.is_err(), "the request failed due to timeout");
    }
}
