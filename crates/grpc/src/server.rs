//! gRPC server implementation.

use std::net::SocketAddr;
use std::time::Duration;

use tokio::sync::oneshot;
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
}

/// Handle to a running gRPC server.
///
/// This handle can be used to get the server's address and to stop the server.
#[derive(Debug)]
pub struct GrpcServerHandle {
    /// The actual address that the server is bound to.
    addr: SocketAddr,
    /// Sender to signal server shutdown.
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl GrpcServerHandle {
    /// Returns the socket address the server is listening on.
    pub fn addr(&self) -> &SocketAddr {
        &self.addr
    }

    /// Stops the server without waiting for it to fully stop.
    pub fn stop(&mut self) -> Result<(), Error> {
        if let Some(tx) = self.shutdown_tx.take() {
            tx.send(()).map_err(|_| Error::AlreadyStopped)?;
        }
        Ok(())
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
    /// This method spawns the server on a new Tokio task and returns a handle
    /// that can be used to manage the server.
    pub async fn start(&self, addr: SocketAddr) -> Result<GrpcServerHandle, Error> {
        // Build reflection service for tooling support (grpcurl, Postman, etc.)
        let reflection_service = tonic_reflection::server::Builder::configure()
            .register_encoded_file_descriptor_set(FILE_DESCRIPTOR_SET)
            .build()
            .map_err(|e| Error::ReflectionBuild(e.to_string()))?;

        // Create shutdown channel
        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        let mut builder = Server::builder().timeout(self.timeout);
        let server = builder.add_routes(self.routes.clone()).add_service(reflection_service);

        // Start the server with graceful shutdown
        let server_future = server.serve_with_shutdown(addr, async {
            let _ = shutdown_rx.await;
        });

        tokio::spawn(async move {
            if let Err(error) = server_future.await {
                error!(target: "grpc", %error, "gRPC server error");
            }
        });

        info!(target: "grpc", %addr, "gRPC server started.");

        Ok(GrpcServerHandle { addr, shutdown_tx: Some(shutdown_tx) })
    }
}

impl Default for GrpcServer {
    fn default() -> Self {
        Self::new()
    }
}
