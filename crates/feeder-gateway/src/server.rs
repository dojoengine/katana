use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::routing::get;
use axum::Router;
use katana_core::backend::Backend;
use katana_executor::implementation::blockifier::BlockifierFactory;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tower::ServiceBuilder;
use tower_http::cors::{Any, CorsLayer};
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;
use tracing::{error, info};

use crate::handlers::{self, AppState};

/// Default timeout for feeder gateway requests
pub const DEFAULT_FEEDER_GATEWAY_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("feeder gateway server has already been stopped")]
    AlreadyStopped,
}

/// The feeder gateway server handle.
#[derive(Debug)]
pub struct FeederGatewayServerHandle {
    /// The actual address that the server is bound to.
    addr: SocketAddr,
    /// Handle to stop the server.
    handle: ServerHandle,
}

impl FeederGatewayServerHandle {
    /// Tell the server to stop without waiting for the server to stop.
    pub fn stop(&self) -> Result<(), Error> {
        self.handle.stop()
    }

    /// Wait until the server has stopped.
    ///
    /// Returns a future that resolves when the server has fully stopped.
    pub async fn stopped(&self) {
        self.handle.stopped().await
    }

    /// Returns the socket address the server is listening on.
    pub fn addr(&self) -> &SocketAddr {
        &self.addr
    }
}

/// The feeder gateway server.
#[derive(Debug)]
pub struct FeederGatewayServer {
    timeout: Duration,
    backend: Arc<Backend<BlockifierFactory>>,
}

impl FeederGatewayServer {
    /// Create a new feeder gateway server.
    pub fn new(backend: Arc<Backend<BlockifierFactory>>) -> Self {
        Self { backend, timeout: DEFAULT_FEEDER_GATEWAY_TIMEOUT }
    }

    /// Set the request timeout. Default is 30 seconds.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Start the feeder gateway server.
    pub async fn start(&self, addr: SocketAddr) -> Result<FeederGatewayServerHandle, Error> {
        let listener = TcpListener::bind(addr).await?;

        let app = self.create_app();
        let actual_addr = listener.local_addr()?;
        let (server_handle, stop_handle) = stop_channel();

        tokio::spawn(async move {
            let server = axum::serve(listener, app).with_graceful_shutdown(async {
                stop_handle.shutdown().await;
            });

            if let Err(err) = server.await {
                error!(target: "feeder_gateway", error = ?err, "Feeder gateway server error.");
            }
        });

        info!(target: "feeder_gateway", addr = %actual_addr, "Feeder gateway server started.");

        Ok(FeederGatewayServerHandle { addr: actual_addr, handle: server_handle })
    }

    /// Create the Axum application with all routes configured
    fn create_app(&self) -> Router {
        // Create shared application state
        let state = AppState { backend: self.backend.clone() };

        let middleware = ServiceBuilder::new()
            .layer(TraceLayer::new_for_http())
            .layer(CorsLayer::new().allow_origin(Any).allow_headers(Any).allow_methods(Any))
            .layer(TimeoutLayer::new(self.timeout));

        Router::new()
            .layer(middleware)
            .route("/feeder_gateway/get_block", get(handlers::get_block))
            .route("/feeder_gateway/get_state_update", get(handlers::get_state_update))
            .route("/feeder_gateway/get_class_by_hash", get(handlers::get_class_by_hash))
            .route(
                "/feeder_gateway/get_compiled_class_by_class_hash",
                get(handlers::get_compiled_class_by_class_hash),
            )
            .with_state(state)
    }
}

/// Server handle.
///
/// When all [`StopHandle`]'s have been `dropped` or `stop` has been called
/// the server will be stopped.
#[derive(Debug, Clone)]
struct ServerHandle(Arc<watch::Sender<()>>);

impl ServerHandle {
    /// Create a new server handle.
    pub(crate) fn new(tx: watch::Sender<()>) -> Self {
        Self(Arc::new(tx))
    }

    /// Tell the server to stop without waiting for the server to stop.
    fn stop(&self) -> Result<(), Error> {
        self.0.send(()).map_err(|_| Error::AlreadyStopped)
    }

    /// Wait for the server to stop.
    async fn stopped(self) {
        self.0.closed().await
    }

    /// Check if the server has been stopped.
    fn is_stopped(&self) -> bool {
        self.0.is_closed()
    }
}

/// Represent a stop handle which is a wrapper over a `multi-consumer receiver`
/// and cloning [`StopHandle`] will get a separate instance of the underlying receiver.
#[derive(Debug, Clone)]
struct StopHandle(watch::Receiver<()>);

impl StopHandle {
    /// Create a new stop handle.
    fn new(rx: watch::Receiver<()>) -> Self {
        Self(rx)
    }

    /// A future that resolves when server has been stopped
    /// it consumes the stop handle.
    async fn shutdown(mut self) {
        let _ = self.0.changed().await;
    }
}

/// Create channel to determine whether
/// the server shall continue to run or not.
fn stop_channel() -> (ServerHandle, StopHandle) {
    let (tx, rx) = tokio::sync::watch::channel(());
    (ServerHandle::new(tx), StopHandle::new(rx))
}
