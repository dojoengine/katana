# HTTPS Support for Katana RPC Server

The Katana RPC server supports HTTPS/TLS connections for secure communication. This document describes how to enable and configure HTTPS support.

## Overview

The HTTPS implementation uses a proxy architecture:
1. The jsonrpsee RPC server runs on localhost
2. A TLS proxy accepts HTTPS connections and forwards them to the RPC server
3. This approach allows HTTPS support without modifying jsonrpsee internals

## Using HTTPS with Katana CLI

### Auto-Generated Certificates (Easiest)

The simplest way to enable HTTPS for development:

```bash
# Automatically generate and use self-signed certificates
katana --https
```

This will:
- Generate self-signed certificates in `.katana/tls/`
- Start HTTPS server on `https://127.0.0.1:5050`
- Reuse the certificates on subsequent runs

### Manual Certificates

You can also provide your own certificate files:

```bash
# Generate certificates first
openssl genrsa -out key.pem 2048
openssl req -new -x509 -key key.pem -out cert.pem -days 365 \
  -subj "/CN=localhost" \
  -addext "subjectAltName=DNS:localhost,IP:127.0.0.1"

# Start Katana with HTTPS
katana --http.tls-cert cert.pem --http.tls-key key.pem
```

The server will start on `https://127.0.0.1:5050` by default (or the port specified with `--http.port`).

### Additional Options

You can combine TLS options with other server configuration:

```bash
# With auto-generated certificates
katana --https --http.port 8443 --http.addr 0.0.0.0

# With manual certificates
katana \
  --http.tls-cert cert.pem \
  --http.tls-key key.pem \
  --http.port 8443 \
  --http.addr 0.0.0.0
```

This will start an HTTPS server on `https://0.0.0.0:8443`.

## Quick Start

### 1. Generate TLS Certificates

For development and testing, you can create self-signed certificates using OpenSSL:

```bash
# Generate a private key
openssl genrsa -out key.pem 2048

# Generate a self-signed certificate valid for 365 days
openssl req -new -x509 -key key.pem -out cert.pem -days 365 \
  -subj "/CN=localhost" \
  -addext "subjectAltName=DNS:localhost,IP:127.0.0.1"
```

**Important:** Self-signed certificates should only be used for development. For production, use certificates from a trusted Certificate Authority (CA) like Let's Encrypt.

### 2. Configure the RPC Server

```rust
use katana_rpc_server::tls::TlsConfig;
use katana_rpc_server::RpcServer;
use jsonrpsee::RpcModule;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create your RPC module
    let module = RpcModule::new(());

    // Configure TLS
    let tls_config = TlsConfig::new("cert.pem", "key.pem");

    // Start the HTTPS server
    let server = RpcServer::new()
        .tls(tls_config)
        .module(module)?
        .start("0.0.0.0:3000".parse()?)
        .await?;

    println!("HTTPS server running at: https://{}", server.addr());

    // Keep server running
    std::future::pending::<()>().await;

    Ok(())
}
```

### 3. Test the Connection

```bash
# Test with curl (use -k to skip certificate verification for self-signed certs)
curl -k -X POST https://localhost:3000 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"your_method","params":[],"id":1}'
```

## Configuration Options

### TLS Configuration

The `TlsConfig` struct requires two parameters:
- **Certificate path**: Path to the PEM-encoded certificate file
- **Private key path**: Path to the PEM-encoded private key file

```rust
let tls_config = TlsConfig::new("path/to/cert.pem", "path/to/key.pem");
```

### Supported Features

- ✅ TLS 1.2 and TLS 1.3
- ✅ HTTP/1.1 and HTTP/2 (via ALPN)
- ✅ All standard RPC server features (CORS, metrics, health checks, etc.)
- ✅ WebSocket connections over TLS
- ✅ Compatible with the Explorer UI

## Production Deployment

For production deployments, we recommend:

1. **Use valid certificates from a trusted CA**
   - Let's Encrypt provides free certificates
   - Use certbot or similar tools for automatic renewal

2. **Alternative: Use a reverse proxy**
   - nginx, Caddy, or HAProxy can handle TLS termination
   - This approach may be more performant and feature-rich
   - The RPC server can run on HTTP behind the proxy

### Example: nginx reverse proxy

```nginx
server {
    listen 443 ssl http2;
    server_name your-domain.com;

    ssl_certificate /path/to/cert.pem;
    ssl_certificate_key /path/to/key.pem;

    location / {
        proxy_pass http://127.0.0.1:5050;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
    }
}
```

## Security Considerations

1. **Certificate Management**
   - Keep private keys secure and never commit them to version control
   - Use strong file permissions (e.g., `chmod 600 key.pem`)
   - Implement automatic certificate renewal for production

2. **TLS Configuration**
   - The server uses secure defaults (TLS 1.2+)
   - ALPN is enabled for HTTP/2 support
   - Consider using a security scanner to verify your TLS configuration

3. **Self-Signed Certificates**
   - Only use for development and testing
   - Clients will need to skip certificate verification or manually trust the certificate
   - Not suitable for production environments

## Troubleshooting

### Certificate Loading Errors

```
Error: failed to open certificate file: No such file or directory
```

**Solution:** Verify that the certificate and key files exist and the paths are correct.

### TLS Handshake Failures

```
Error: TLS handshake failed
```

**Solutions:**
- Verify the certificate and private key match
- Ensure the certificate is valid and not expired
- Check that the client supports the TLS versions configured

### Connection Refused

```
Error: Connection refused
```

**Solutions:**
- Verify the server is running and bound to the correct address
- Check firewall rules allow HTTPS traffic (port 443 or custom port)
- Ensure the port is not already in use

## Example

See `examples/https_server.rs` for a complete working example:

```bash
cargo run --example https_server
```

## Additional Resources

- [rustls documentation](https://docs.rs/rustls/)
- [Let's Encrypt](https://letsencrypt.org/)
- [Mozilla SSL Configuration Generator](https://ssl-config.mozilla.org/)
