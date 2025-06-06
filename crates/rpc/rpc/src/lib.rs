//! RPC implementations.

#![allow(clippy::blocks_in_conditions)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use std::net::SocketAddr;
use std::time::Duration;

use jsonrpsee::core::middleware::RpcServiceBuilder;
use jsonrpsee::core::{RegisterMethodError, TEN_MB_SIZE_BYTES};
use jsonrpsee::server::{Server, ServerConfig, ServerHandle};
use jsonrpsee::RpcModule;
use katana_log::gcloud::GoogleStackDriverMakeSpan;
use tower::ServiceBuilder;
use tower_http::trace::TraceLayer;
use tracing::info;

#[cfg(feature = "cartridge")]
pub mod cartridge;

pub mod cors;
pub mod dev;
pub mod health;
pub mod metrics;
pub mod starknet;

mod utils;
use cors::Cors;
use health::HealthCheck;
use metrics::RpcServerMetricsLayer;

/// The default maximum number of concurrent RPC connections.
pub const DEFAULT_RPC_MAX_CONNECTIONS: u32 = 100;
/// The default maximum size in bytes for an RPC request body.
pub const DEFAULT_MAX_REQUEST_BODY_SIZE: u32 = TEN_MB_SIZE_BYTES;
/// The default maximum size in bytes for an RPC response body.
pub const DEFAULT_MAX_RESPONSE_BODY_SIZE: u32 = TEN_MB_SIZE_BYTES;
/// The default timeout for an RPC request.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Jsonrpsee(#[from] jsonrpsee::types::ErrorObjectOwned),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    RegisterMethod(#[from] RegisterMethodError),

    #[error("RPC server has already been stopped")]
    AlreadyStopped,
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
}

#[derive(Debug)]
pub struct RpcServer {
    metrics: bool,
    cors: Option<Cors>,
    health_check: bool,
    explorer: bool,

    module: RpcModule<()>,
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
            module: RpcModule::new(()),
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

    /// Adds a new RPC module to the server.
    ///
    /// This can be chained with other calls to `module` to add multiple modules.
    ///
    /// # Example
    ///
    /// ```rust
    /// let server = RpcServer::new().module(module_a()).unwrap().module(module_b()).unwrap();
    /// ```
    pub fn module(mut self, module: RpcModule<()>) -> Result<Self, Error> {
        self.module.merge(module)?;
        Ok(self)
    }

    pub async fn start(&self, addr: SocketAddr) -> Result<RpcServerHandle, Error> {
        let mut modules = self.module.clone();

        let health_check_proxy = if self.health_check {
            modules.merge(HealthCheck)?;
            Some(HealthCheck::proxy())
        } else {
            None
        };

        #[cfg(feature = "explorer")]
        let explorer_layer = if self.explorer {
            let layer = katana_explorer::ExplorerLayer::new(String::new()).unwrap();
            Some(layer)
        } else {
            None
        };

        let rpc_metrics = self.metrics.then(|| RpcServerMetricsLayer::new(&modules));
        let http_tracer = TraceLayer::new_for_http().make_span_with(GoogleStackDriverMakeSpan);

        let http_middleware = ServiceBuilder::new()
            .layer(http_tracer)
            .option_layer(self.cors.clone())
            .option_layer(health_check_proxy)
            .timeout(self.timeout);

        #[cfg(feature = "explorer")]
        let http_middleware = http_middleware.option_layer(explorer_layer);

        let rpc_middleware = RpcServiceBuilder::new().option_layer(rpc_metrics).rpc_logger(0);

        let cfg = ServerConfig::builder()
            .max_connections(self.max_connections)
            .max_request_body_size(self.max_request_body_size)
            .max_response_body_size(self.max_response_body_size)
            .build();

        let server = Server::builder()
            .set_http_middleware(http_middleware)
            .set_rpc_middleware(rpc_middleware)
            .set_config(cfg)
            .build(addr)
            .await?;

        let actual_addr = server.local_addr()?;
        let handle = server.start(modules);

        let handle = RpcServerHandle { handle, addr: actual_addr };

        // The socket address that we log out must be from the RPC handle, in the case that the
        // `addr` passed to this method has port number 0. As the 0 port will be resolved to
        // a free port during the call to `ServerBuilder::build(addr)`.

        info!(target: "rpc", addr = %handle.addr, "RPC server started.");

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
