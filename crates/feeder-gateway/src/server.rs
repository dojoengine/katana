use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::routing::get;
use axum::Router;
use katana_core::backend::Backend;
use katana_executor::implementation::blockifier::BlockifierFactory;
use tokio::net::TcpListener;
use tower::ServiceBuilder;
use tower_http::cors::{Any, CorsLayer};
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;
use tracing::info;

use crate::handlers::{self, AppState};

/// Default port for the feeder gateway server
pub const DEFAULT_FEEDER_GATEWAY_PORT: u16 = 5051;
/// Default timeout for feeder gateway requests
pub const DEFAULT_FEEDER_GATEWAY_TIMEOUT: Duration = Duration::from_secs(30);

/// The feeder gateway server handle.
#[derive(Debug)]
pub struct FeederGatewayServerHandle {
    /// The actual address that the server is bound to.
    addr: SocketAddr,
    /// Handle to stop the server.
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl FeederGatewayServerHandle {
    /// Tell the server to stop without waiting for the server to stop.
    pub fn stop(&mut self) -> Result<(), Error> {
        if let Some(tx) = self.shutdown_tx.take() {
            tx.send(()).map_err(|_| Error::AlreadyStopped)
        } else {
            Err(Error::AlreadyStopped)
        }
    }

    /// Returns the socket address the server is listening on.
    pub fn addr(&self) -> &SocketAddr {
        &self.addr
    }
}

/// Configuration for the feeder gateway server.
#[derive(Debug, Clone)]
pub struct FeederGatewayConfig {
    pub timeout: Duration,
}

impl Default for FeederGatewayConfig {
    fn default() -> Self {
        Self { timeout: DEFAULT_FEEDER_GATEWAY_TIMEOUT }
    }
}

/// The feeder gateway server.
#[derive(Debug)]
pub struct FeederGatewayServer {
    config: FeederGatewayConfig,
    backend: Arc<Backend<BlockifierFactory>>,
}

impl FeederGatewayServer {
    /// Create a new feeder gateway server.
    pub fn new(backend: Arc<Backend<BlockifierFactory>>) -> Self {
        Self { config: FeederGatewayConfig::default(), backend }
    }

    /// Set the server configuration.
    pub fn config(mut self, config: FeederGatewayConfig) -> Self {
        self.config = config;
        self
    }

    /// Set the request timeout. Default is 30 seconds.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.config.timeout = timeout;
        self
    }

    /// Start the feeder gateway server.
    pub async fn start(self, addr: SocketAddr) -> Result<FeederGatewayServerHandle, Error> {
        let listener = TcpListener::bind(addr).await?;

        // Create the Axum application with routes
        let app = self.create_app();

        let actual_addr = listener.local_addr()?;
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

        // Start the axum server
        tokio::spawn(async move {
            let server = axum::serve(listener, app).with_graceful_shutdown(async {
                shutdown_rx.await.ok();
            });

            if let Err(e) = server.await {
                tracing::error!("Feeder gateway server error: {}", e);
            }
        });

        info!(target: "feeder_gateway", addr = %actual_addr, "Feeder gateway server started.");

        Ok(FeederGatewayServerHandle { addr: actual_addr, shutdown_tx: Some(shutdown_tx) })
    }

    /// Create the Axum application with all routes configured
    fn create_app(self) -> Router {
        // Create shared application state
        let state = AppState { backend: self.backend };

        let middleware = ServiceBuilder::new()
            .layer(TraceLayer::new_for_http())
            .layer(CorsLayer::new().allow_origin(Any).allow_headers(Any).allow_methods(Any))
            .layer(TimeoutLayer::new(self.config.timeout));

        Router::new()
            .layer(middleware)
            .with_state(state)
            .route("/feeder_gateway/get_block", get(handlers::get_block))
            .route("/feeder_gateway/get_state_update", get(handlers::get_state_update))
            .route("/feeder_gateway/get_class_by_hash", get(handlers::get_class_by_hash))
            .route(
                "/feeder_gateway/get_compiled_class_by_class_hash",
                get(handlers::get_compiled_class_by_class_hash),
            )
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("server has already been stopped")]
    AlreadyStopped,
}
