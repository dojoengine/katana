//! TLS proxy that forwards HTTPS requests to the RPC server.

use std::net::SocketAddr;

use hyper::service::service_fn;
use hyper::{Body, Client, Request, Response};
use tokio::task::JoinHandle;
use tracing::{debug, error, info};

use crate::tls::{TlsConfig, TlsListener};

/// A TLS proxy that accepts HTTPS connections and forwards them to a backend HTTP server.
pub struct TlsProxy {
    backend_addr: SocketAddr,
    listener: TlsListener,
    is_self_signed: bool,
}

impl TlsProxy {
    /// Creates a new TLS proxy.
    ///
    /// # Arguments
    ///
    /// * `listen_addr` - The address to listen for HTTPS connections
    /// * `backend_addr` - The address of the backend HTTP server (jsonrpsee)
    /// * `tls_config` - TLS configuration (certificate and private key)
    pub async fn new(
        listen_addr: SocketAddr,
        backend_addr: SocketAddr,
        tls_config: TlsConfig,
    ) -> Result<Self, crate::Error> {
        let is_self_signed = tls_config.is_self_signed;
        let listener = TlsListener::bind(listen_addr, tls_config).await?;
        Ok(Self { backend_addr, listener, is_self_signed })
    }

    /// Returns the local address that this proxy is bound to.
    pub fn local_addr(&self) -> Result<SocketAddr, std::io::Error> {
        self.listener.local_addr()
    }

    /// Starts the TLS proxy server.
    ///
    /// This method spawns a task that accepts TLS connections and forwards them to the backend.
    pub fn start(self) -> JoinHandle<()> {
        tokio::spawn(async move {
            let listen_addr = self.listener.local_addr().unwrap();
            info!(
                target: "rpc",
                listen = %listen_addr,
                backend = %self.backend_addr,
                "TLS proxy started"
            );

            // Print helpful connection info for self-signed certificates
            if self.is_self_signed {
                info!(target: "rpc", "");
                info!(target: "rpc", "🔒 HTTPS with self-signed certificate enabled");
                info!(target: "rpc", "");
                info!(target: "rpc", "To connect with curl, use the -k flag:");
                info!(target: "rpc", "  curl -k https://{}", listen_addr);
                info!(target: "rpc", "");
                info!(target: "rpc", "For other clients, disable certificate verification:");
                info!(target: "rpc", "  - Rust reqwest: .danger_accept_invalid_certs(true)");
                info!(target: "rpc", "  - Python requests: verify=False");
                info!(target: "rpc", "  - JavaScript fetch: rejectUnauthorized: false");
                info!(target: "rpc", "");
            }

            let is_self_signed = self.is_self_signed;
            loop {
                match self.listener.accept().await {
                    Ok((stream, peer_addr)) => {
                        debug!(peer = %peer_addr, "Accepted TLS connection");
                        let backend_addr = self.backend_addr;

                        tokio::spawn(async move {
                            let service =
                                service_fn(move |req| Self::proxy_request(req, backend_addr));

                            // Use hyper 0.14's server API with the TLS stream
                            let http = hyper::server::conn::Http::new();
                            if let Err(e) = http.serve_connection(stream, service).await {
                                error!(peer = %peer_addr, error = %e, "Error serving connection");
                            }
                        });
                    }
                    Err(e) => {
                        // Suppress expected certificate errors for self-signed certs
                        if is_self_signed
                            && (e.to_string().contains("CertificateUnknown")
                                || e.to_string().contains("UnknownCA")
                                || e.to_string().contains("certificate"))
                        {
                            debug!(error = %e, "TLS connection failed (expected with self-signed certificates)");
                        } else {
                            error!(error = %e, "Failed to accept TLS connection");
                        }
                    }
                }
            }
        })
    }

    /// Proxies a request to the backend server.
    async fn proxy_request(
        mut req: Request<Body>,
        backend_addr: SocketAddr,
    ) -> Result<Response<Body>, hyper::Error> {
        let client = Client::new();

        // Update the request URI to point to the backend
        let uri_string = format!(
            "http://{}{}",
            backend_addr,
            req.uri().path_and_query().map(|pq| pq.as_str()).unwrap_or("/")
        );

        *req.uri_mut() = uri_string.parse().unwrap();

        // Forward the request to the backend
        client.request(req).await
    }
}
