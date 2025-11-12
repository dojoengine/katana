# HTTPS Support in Katana

Katana supports running the RPC server over HTTPS for secure communication. This guide covers how to enable HTTPS mode when running Katana.

## Quick Start (Development)

The easiest way to enable HTTPS for development is to use the `--https` flag, which automatically generates self-signed certificates:

```bash
katana --https
```

This will:
1. Generate a self-signed certificate and private key in `.katana/tls/`
2. Start the HTTPS server on `https://127.0.0.1:5050`
3. Reuse the same certificates on subsequent runs

**Note:** The `--https` flag is for development only. For production, use proper certificates (see below).

## Enable HTTPS with Custom Certificates

To enable HTTPS with your own certificates, provide both a TLS certificate and private key when starting Katana:

```bash
katana --http.tls-cert /path/to/cert.pem --http.tls-key /path/to/key.pem
```

**Note:** Both `--http.tls-cert` and `--http.tls-key` must be provided together. Katana will not start if only one is specified.

## Generate Development Certificates

For local development and testing, you can generate self-signed certificates:

```bash
# Generate a private key
openssl genrsa -out key.pem 2048

# Generate a self-signed certificate valid for 365 days
openssl req -new -x509 -key key.pem -out cert.pem -days 365 \
  -subj "/CN=localhost" \
  -addext "subjectAltName=DNS:localhost,IP:127.0.0.1"
```

Then start Katana:

```bash
katana --http.tls-cert cert.pem --http.tls-key key.pem
```

## Testing HTTPS Connections

### With curl

When using self-signed certificates, you'll need to use the `-k` flag to skip certificate verification:

```bash
curl -k https://localhost:5050 \
  -X POST \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"starknet_chainId","params":[],"id":1}'
```

### With starknet-rs

```rust
use starknet::providers::{jsonrpc::HttpTransport, JsonRpcClient};
use url::Url;

// For self-signed certificates in development
let url = Url::parse("https://localhost:5050")?;
let provider = JsonRpcClient::new(HttpTransport::new(url));
```

## Configuration Options

### Port Configuration

By default, HTTPS runs on port 5050. Change it with:

```bash
katana --http.tls-cert cert.pem --http.tls-key key.pem --http.port 8443
```

### Network Interface

Bind to a specific interface:

```bash
# Bind to all interfaces
katana --http.tls-cert cert.pem --http.tls-key key.pem --http.addr 0.0.0.0

# Bind to specific IP
katana --http.tls-cert cert.pem --http.tls-key key.pem --http.addr 192.168.1.100
```

### Combined Example (Development)

Using auto-generated certificates with custom configuration:

```bash
katana \
  --https \
  --http.addr 0.0.0.0 \
  --http.port 8443 \
  --http.cors_origins "*"
```

### Combined Example (Production)

Using your own certificates:

```bash
katana \
  --http.tls-cert cert.pem \
  --http.tls-key key.pem \
  --http.addr 0.0.0.0 \
  --http.port 443 \
  --http.cors_origins "*"
```

## Production Deployment

For production use, we recommend:

1. **Use Valid Certificates**: Obtain certificates from a trusted Certificate Authority like Let's Encrypt
2. **Certificate Renewal**: Implement automatic certificate renewal (e.g., with certbot)
3. **Reverse Proxy**: Consider using nginx or Caddy in front of Katana for additional features:
   - Rate limiting
   - DDoS protection
   - Advanced TLS configuration
   - Load balancing

### Example: nginx Reverse Proxy

```nginx
server {
    listen 443 ssl http2;
    server_name your-domain.com;

    ssl_certificate /etc/letsencrypt/live/your-domain.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/your-domain.com/privkey.pem;

    location / {
        proxy_pass http://127.0.0.1:5050;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
```

Then run Katana on HTTP locally:

```bash
katana --http.addr 127.0.0.1 --http.port 5050
```

## Security Considerations

1. **Certificate Files**: Protect your private key with proper file permissions:
   ```bash
   chmod 600 key.pem
   chmod 644 cert.pem
   ```

2. **Self-Signed Certificates**: Only use for development. Clients will need to manually trust the certificate or skip verification.

3. **Keep Certificates Updated**: Expired certificates will prevent clients from connecting.

4. **Network Security**: When binding to `0.0.0.0`, ensure your firewall rules are properly configured.

## Troubleshooting

### Certificate Loading Errors

```
Error: failed to open certificate file
```

**Solution**: Verify the file paths are correct and the files exist:
```bash
ls -la cert.pem key.pem
```

### TLS Handshake Failures

```
Error: TLS handshake failed
```

**Solutions**:
- Verify the certificate and private key match
- Check that the certificate is not expired
- Ensure the certificate includes the correct domain/IP in Subject Alternative Names

### Connection Refused

```
Error: Connection refused
```

**Solutions**:
- Verify Katana is running: `ps aux | grep katana`
- Check the correct port is being used
- Verify firewall rules allow the connection

## See Also

- [RPC Server Documentation](../crates/rpc/rpc-server/HTTPS.md) - Detailed technical documentation
- [Example Code](../crates/rpc/rpc-server/examples/https_server.rs) - Programmatic usage example
