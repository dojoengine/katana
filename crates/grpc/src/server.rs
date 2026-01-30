//! gRPC server implementation.

use std::net::SocketAddr;

use tokio::sync::oneshot;
use tonic::transport::Server;
use tracing::info;

use crate::config::GrpcConfig;
use crate::handlers::StarknetHandler;
use crate::protos::starknet::starknet_server::StarknetServer;
use crate::protos::starknet::starknet_trace_server::StarknetTraceServer;
use crate::protos::starknet::starknet_write_server::StarknetWriteServer;
use crate::protos::starknet::FILE_DESCRIPTOR_SET;

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
    config: GrpcConfig,
}

impl GrpcServer {
    /// Creates a new gRPC server builder with the given configuration.
    pub fn new(config: GrpcConfig) -> Self {
        Self { config }
    }

    /// Starts the gRPC server.
    ///
    /// This method spawns the server on a new Tokio task and returns a handle
    /// that can be used to manage the server.
    pub async fn start<H>(self, handler: H) -> Result<GrpcServerHandle, Error>
    where
        H: Clone + Send + Sync + 'static,
        StarknetHandler<H>:
            crate::protos::starknet::starknet_server::Starknet
                + crate::protos::starknet::starknet_write_server::StarknetWrite
                + crate::protos::starknet::starknet_trace_server::StarknetTrace,
    {
        let addr = self.config.socket_addr();

        // Build reflection service for tooling support (grpcurl, Postman, etc.)
        let reflection_service = tonic_reflection::server::Builder::configure()
            .register_encoded_file_descriptor_set(FILE_DESCRIPTOR_SET)
            .build_v1()
            .map_err(|e| Error::ReflectionBuild(e.to_string()))?;

        // Create the service handler
        let starknet_handler = StarknetHandler::new(handler);

        // Create shutdown channel
        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        // Build the server
        let mut builder = Server::builder();

        // Configure timeout if specified
        if let Some(timeout) = self.config.timeout {
            builder = builder.timeout(timeout);
        }

        // Configure max concurrent connections if specified
        if let Some(max_connections) = self.config.max_connections {
            builder = builder.concurrency_limit_per_connection(max_connections as usize);
        }

        let server = builder
            .add_service(reflection_service)
            .add_service(StarknetServer::new(starknet_handler.clone()))
            .add_service(StarknetWriteServer::new(starknet_handler.clone()))
            .add_service(StarknetTraceServer::new(starknet_handler));

        // Start the server with graceful shutdown
        let server_future = server.serve_with_shutdown(addr, async {
            let _ = shutdown_rx.await;
        });

        // Spawn the server task
        tokio::spawn(async move {
            if let Err(e) = server_future.await {
                tracing::error!(error = %e, "gRPC server error");
            }
        });

        info!(target: "grpc", %addr, "gRPC server started.");

        Ok(GrpcServerHandle { addr, shutdown_tx: Some(shutdown_tx) })
    }
}

impl Default for GrpcServer {
    fn default() -> Self {
        Self::new(GrpcConfig::default())
    }
}
