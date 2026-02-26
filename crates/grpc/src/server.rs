//! gRPC server implementation.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;
use tonic::transport::server::Routes;
use tonic::transport::Server;
use tracing::{error, info};

use crate::protos::starknet::FILE_DESCRIPTOR_SET;

/// The default timeout for an request.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);

/// Error type for gRPC server operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Transport error from tonic.
    #[error(transparent)]
    Transport(#[from] tonic::transport::Error),

    /// Reflection service build error.
    #[error("Failed to build reflection service: {0}")]
    ReflectionBuild(String),

    /// Server has already been stopped.
    #[error("gRPC server has already been stopped")]
    AlreadyStopped,

    /// IO error from binding the TCP listener.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// Error from creating the TCP incoming stream.
    #[error("Failed to create TCP incoming stream: {0}")]
    Incoming(String),
}

/// Handle to a running gRPC server.
///
/// This handle can be used to get the server's address and to stop the server.
#[derive(Debug, Clone)]
pub struct GrpcServerHandle {
    /// The actual address that the server is bound to.
    addr: SocketAddr,
    /// Sender to signal server shutdown.
    shutdown_tx: Arc<watch::Sender<()>>,
}

impl GrpcServerHandle {
    /// Returns the socket address the server is listening on.
    pub fn addr(&self) -> &SocketAddr {
        &self.addr
    }

    /// Stops the server without waiting for it to fully stop.
    pub fn stop(&self) -> Result<(), Error> {
        self.shutdown_tx.send(()).map_err(|_| Error::AlreadyStopped)
    }

    /// Wait until the server has stopped.
    pub async fn stopped(&self) {
        self.shutdown_tx.closed().await
    }

    /// Returns true if the server has stopped.
    pub fn is_stopped(&self) -> bool {
        self.shutdown_tx.is_closed()
    }
}

/// Builder for the gRPC server.
#[derive(Debug, Clone)]
pub struct GrpcServer {
    routes: Routes,
    /// Request timeout.
    timeout: Duration,
}

impl GrpcServer {
    /// Creates a new gRPC server builder with the given configuration.
    pub fn new() -> Self {
        Self { routes: Routes::default(), timeout: DEFAULT_TIMEOUT }
    }

    /// Set the timeout for the server. Default is 20 seconds.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn service<S>(mut self, service: S) -> Self
    where
        S: tower_service::Service<
                http::Request<tonic::transport::Body>,
                Response = http::Response<tonic::body::BoxBody>,
                Error = std::convert::Infallible,
            > + tonic::server::NamedService
            + Clone
            + Send
            + 'static,
        S::Future: Send + 'static,
        S::Error: Into<Box<dyn std::error::Error + Send + Sync>> + Send,
    {
        self.routes = self.routes.add_service(service);
        self
    }

    /// Starts the gRPC server.
    ///
    /// This method binds to the given address, spawns the server on a new Tokio task,
    /// and returns a handle that can be used to manage the server. The server is
    /// guaranteed to be listening for connections when this method returns.
    pub async fn start(&self, addr: SocketAddr) -> Result<GrpcServerHandle, Error> {
        // Build reflection service for tooling support (grpcurl, Postman, etc.)
        let reflection_service = tonic_reflection::server::Builder::configure()
            .register_encoded_file_descriptor_set(FILE_DESCRIPTOR_SET)
            .build()
            .map_err(|e| Error::ReflectionBuild(e.to_string()))?;

        // Create shutdown channel
        let (shutdown_tx, mut shutdown_rx) = watch::channel(());

        // Bind the TCP listener BEFORE spawning the server task. This ensures:
        // 1. The port is resolved (important when addr uses port 0 for auto-assignment)
        // 2. The server is accepting connections when this method returns
        let listener = tokio::net::TcpListener::bind(addr).await?;
        let actual_addr = listener.local_addr()?;

        let incoming = tonic::transport::server::TcpIncoming::from_listener(listener, true, None)
            .map_err(|e| Error::Incoming(e.to_string()))?;

        let mut builder = Server::builder().timeout(self.timeout);
        let server = builder.add_routes(self.routes.clone()).add_service(reflection_service);

        // Start the server with the already-bound listener
        tokio::spawn(async move {
            if let Err(error) =
                server.serve_with_incoming_shutdown(incoming, async move {
                    let _ = shutdown_rx.changed().await;
                }).await
            {
                error!(target: "grpc", %error, "gRPC server error");
            }
        });

        info!(target: "grpc", addr = %actual_addr, "gRPC server started.");

        Ok(GrpcServerHandle { addr: actual_addr, shutdown_tx: Arc::new(shutdown_tx) })
    }
}

impl Default for GrpcServer {
    fn default() -> Self {
        Self::new()
    }
}
