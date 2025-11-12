//! Example demonstrating how to start an RPC server with HTTPS support.
//!
//! # Prerequisites
//!
//! You need to generate a self-signed certificate and private key for testing.
//! You can use OpenSSL to generate them:
//!
//! ```bash
//! # Generate a private key
//! openssl genrsa -out key.pem 2048
//!
//! # Generate a self-signed certificate
//! openssl req -new -x509 -key key.pem -out cert.pem -days 365 \
//!   -subj "/CN=localhost" \
//!   -addext "subjectAltName=DNS:localhost,IP:127.0.0.1"
//! ```
//!
//! # Running the example
//!
//! ```bash
//! cargo run --example https_server
//! ```
//!
//! # Testing the server
//!
//! ```bash
//! # Test with curl (note the -k flag to skip certificate verification for self-signed certs)
//! curl -k -X POST https://localhost:3000 \
//!   -H "Content-Type: application/json" \
//!   -d '{"jsonrpc":"2.0","method":"test_method","params":[],"id":1}'
//! ```

use std::future::pending;

use jsonrpsee::RpcModule;
use katana_rpc_server::tls::TlsConfig;
use katana_rpc_server::RpcServer;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing for logging
    tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).init();

    // Create a simple RPC module with a test method
    let mut module = RpcModule::new(());
    module.register_method("test_method", |_, _, _| {
        Ok::<_, jsonrpsee::types::ErrorObjectOwned>("Hello from HTTPS!")
    })?;

    // Configure TLS with certificate and private key paths
    // Note: You need to generate these files first (see instructions above)
    let tls_config = TlsConfig::new("cert.pem", "key.pem");

    // Create and start the HTTPS server
    let server =
        RpcServer::new().tls(tls_config).module(module)?.start("127.0.0.1:3000".parse()?).await?;

    println!("🔒 HTTPS RPC server started at: https://{}", server.addr());
    println!("Press Ctrl+C to stop the server");

    // Keep the server running
    pending::<()>().await;

    Ok(())
}
