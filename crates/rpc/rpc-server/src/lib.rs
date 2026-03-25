//! RPC implementations.

#![allow(clippy::blocks_in_conditions)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use jsonrpsee::core::middleware::RpcServiceBuilder;
use jsonrpsee::core::{RegisterMethodError, TEN_MB_SIZE_BYTES};
use jsonrpsee::server::{Server, ServerConfig, ServerHandle, StopHandle, TowerServiceBuilder};
use jsonrpsee::{Methods, RpcModule};
use tracing::{error, info};

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

// ---------------------------------------------------------------------------
// RpcRouter
// ---------------------------------------------------------------------------

/// Maps URL path prefixes to JSON-RPC modules.
///
/// The router matches incoming requests by path prefix (first match wins)
/// and dispatches to the associated [`RpcModule`]. Designed after axum's
/// `Router` — all module registration goes through `.route()`.
///
/// ```rust,ignore
/// use jsonrpsee::RpcModule;
///
/// let router = RpcRouter::new()
///     .route("/", v09_module)
///     .route("/rpc/v0_9", v09_module)
///     .route("/rpc/v0_10", v010_module);
/// ```
#[derive(Debug, Default, Clone)]
pub struct RpcRouter {
    routes: Vec<(String, RpcModule<()>)>,
}

impl RpcRouter {
    pub fn new() -> Self {
        Self { routes: Vec::new() }
    }

    /// Register a module at the given path prefix.
    ///
    /// Requests whose URL path starts with `path` are dispatched to this
    /// module. Routes are matched in the order they are registered — first
    /// match wins.
    pub fn route(mut self, path: impl Into<String>, module: RpcModule<()>) -> Self {
        self.routes.push((path.into(), module));
        self
    }

    /// Merge another router's routes into this one.
    pub fn merge(mut self, other: RpcRouter) -> Self {
        self.routes.extend(other.routes);
        self
    }
}

// Allow constructing from a single module (mounts at `/`).
impl From<RpcModule<()>> for RpcRouter {
    fn from(module: RpcModule<()>) -> Self {
        Self::new().route("/", module)
    }
}

// ---------------------------------------------------------------------------
// RpcServerHandle
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// RpcServer
// ---------------------------------------------------------------------------

/// JSON-RPC server with path-based module routing.
///
/// Accepts an [`RpcRouter`] that maps URL paths to modules, and handles
/// server configuration (CORS, timeouts, metrics, etc.).
///
/// ```rust,ignore
/// let router = RpcRouter::new()
///     .route("/", v09_module.clone())
///     .route("/rpc/v0_9", v09_module)
///     .route("/rpc/v0_10", v010_module);
///
/// let handle = RpcServer::new(router)
///     .cors(cors)
///     .health_check(true)
///     .metrics(true)
///     .start(addr)
///     .await?;
/// ```
#[derive(Debug)]
pub struct RpcServer {
    router: RpcRouter,

    metrics: bool,
    cors: Option<Cors>,
    health_check: bool,
    explorer: bool,
    max_connections: u32,
    max_request_body_size: u32,
    max_response_body_size: u32,
    timeout: Duration,
}

impl RpcServer {
    pub fn new(router: impl Into<RpcRouter>) -> Self {
        Self {
            router: router.into(),
            cors: None,
            metrics: false,
            explorer: false,
            health_check: false,
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
    pub fn metrics(mut self, enable: bool) -> Self {
        self.metrics = enable;
        self
    }

    /// Enables health checking endpoint via HTTP `GET /health`.
    pub fn health_check(mut self, enable: bool) -> Self {
        self.health_check = enable;
        self
    }

    /// Enables the embedded explorer UI.
    pub fn explorer(mut self, enable: bool) -> Self {
        self.explorer = enable;
        self
    }

    pub fn cors(mut self, cors: Cors) -> Self {
        self.cors = Some(cors);
        self
    }

    pub async fn start(&self, addr: SocketAddr) -> Result<RpcServerHandle, Error> {
        use futures::FutureExt;
        use jsonrpsee::server::{serve_with_graceful_shutdown, stop_channel};
        use katana_tracing::gcloud::GoogleStackDriverMakeSpan;
        use tokio::net::TcpListener;
        use tower::ServiceBuilder;
        use tower_http::trace::TraceLayer;

        // Prepare health check module
        let health_module: Option<Methods> = if self.health_check {
            let mut m = RpcModule::new(());
            m.merge(HealthCheck)?;
            Some(m.into())
        } else {
            None
        };

        // Convert router to Methods, merging health check and building per-route
        // metrics. Each route gets its own RpcServerMetricsLayer labelled with the
        // path so that method call metrics are distinguishable by route.
        let routes: Vec<(String, Methods, Option<RpcServerMetricsLayer>)> = self
            .router
            .routes
            .iter()
            .map(|(path, module)| {
                let mut m = module.clone();
                if let Some(ref hc) = health_module {
                    let _ = m.merge(hc.clone());
                }

                let metrics = if self.metrics {
                    Some(RpcServerMetricsLayer::new_with_path(module, path))
                } else {
                    None
                };

                (path.clone(), m.into(), metrics)
            })
            .collect();

        // HTTP middleware
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

        let svc_builder = Server::builder()
            .set_http_middleware(http_middleware)
            .set_config(cfg)
            .to_service_builder();

        let listener = TcpListener::bind(addr).await?;
        let actual_addr = listener.local_addr()?;
        let (stop_hdl, server_handle) = stop_channel();

        // Per-connection state.
        #[derive(Clone)]
        struct PerConnection<RpcMiddleware, HttpMiddleware> {
            routes: Arc<Vec<(String, Methods, Option<RpcServerMetricsLayer>)>>,
            stop_handle: StopHandle,
            svc_builder: TowerServiceBuilder<RpcMiddleware, HttpMiddleware>,
        }

        let per_conn = PerConnection {
            svc_builder,
            stop_handle: stop_hdl.clone(),
            routes: Arc::new(routes),
        };

        tokio::spawn(async move {
            loop {
                let stream = tokio::select! {
                    res = listener.accept() => {
                        match res {
                            Ok((stream, _)) => stream,
                            Err(e) => {
                                error!(target: "rpc", "failed to accept connection: {e:?}");
                                continue;
                            }
                        }
                    }

                    _ = per_conn.stop_handle.clone().shutdown() => break,
                };

                let per_conn = per_conn.clone();
                let stop_handle = per_conn.stop_handle.clone();

                let svc = tower::service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
                    let PerConnection { routes, stop_handle, svc_builder } = per_conn.clone();

                    // Route: first prefix match wins. Each route carries its own
                    // metrics layer so that method calls are labelled with the path.
                    let path = req.uri().path();
                    let (methods, rpc_metrics) = routes
                        .iter()
                        .find(|(prefix, _, _)| path.starts_with(prefix.as_str()))
                        .map(|(_, m, metrics)| (m.clone(), metrics.clone()))
                        .unwrap_or_default();

                    let rpc_middleware = RpcServiceBuilder::new()
                        .option_layer(rpc_metrics)
                        .layer(logger::RpcLoggerLayer::new());

                    let mut svc =
                        svc_builder.set_rpc_middleware(rpc_middleware).build(methods, stop_handle);

                    async move { tower::Service::call(&mut svc, req).await }.boxed()
                });

                tokio::spawn(serve_with_graceful_shutdown(stream, svc, stop_handle.shutdown()));
            }
        });

        info!(target: "rpc", addr = %actual_addr, "RPC server started.");

        for (path, _) in &self.router.routes {
            if path != "/" {
                info!(target: "rpc", path = %path, "RPC module mounted.");
            }
        }

        if self.explorer {
            let addr = format!("{}/explorer", actual_addr);
            info!(target: "explorer", %addr, "Explorer started.");
        }

        Ok(RpcServerHandle { handle: server_handle, addr: actual_addr })
    }
}

#[cfg(test)]
mod tests {
    use std::future::pending;
    use std::time::Duration;

    use jsonrpsee::{rpc_params, RpcModule};

    use crate::{RpcRouter, RpcServer};

    #[tokio::test]
    async fn test_rpc_server_timeout() {
        use jsonrpsee::core::client::ClientT;

        let mut module = RpcModule::new(());
        module.register_async_method("test_timeout", |_, _, _| pending::<()>()).unwrap();

        let router = RpcRouter::new().route("/", module);
        let server = RpcServer::new(router).timeout(Duration::from_millis(200));

        let addr = "127.0.0.1:0".parse().unwrap();
        let handle = server.start(addr).await.unwrap();

        let client = handle.http_client().unwrap();
        let result = client.request::<String, _>("test_timeout", rpc_params![]).await;

        assert!(result.is_err(), "the request failed due to timeout");
    }
}
