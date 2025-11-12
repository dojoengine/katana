//! TLS configuration and utilities for HTTPS support.

use std::fs::File;
use std::io::BufReader;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;
use tracing::{debug, error};

/// TLS configuration for HTTPS support.
#[derive(Debug, Clone)]
pub struct TlsConfig {
    /// Path to the TLS certificate file (PEM format).
    pub cert_path: PathBuf,
    /// Path to the TLS private key file (PEM format).
    pub key_path: PathBuf,
    /// Whether this is using self-signed certificates (suppresses expected errors).
    pub is_self_signed: bool,
}

impl TlsConfig {
    /// Creates a new TLS configuration.
    pub fn new(cert_path: impl Into<PathBuf>, key_path: impl Into<PathBuf>) -> Self {
        Self { cert_path: cert_path.into(), key_path: key_path.into(), is_self_signed: false }
    }

    /// Creates a new TLS configuration for self-signed certificates.
    pub fn new_self_signed(cert_path: impl Into<PathBuf>, key_path: impl Into<PathBuf>) -> Self {
        Self { cert_path: cert_path.into(), key_path: key_path.into(), is_self_signed: true }
    }

    /// Loads the certificate chain from the certificate file.
    fn load_certs(&self) -> Result<Vec<CertificateDer<'static>>, crate::Error> {
        let cert_file = File::open(&self.cert_path).map_err(|e| {
            std::io::Error::new(
                e.kind(),
                format!("failed to open certificate file: {}", self.cert_path.display()),
            )
        })?;

        let mut reader = BufReader::new(cert_file);
        let certs =
            rustls_pemfile::certs(&mut reader).collect::<Result<Vec<_>, _>>().map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("failed to parse certificate: {e}"),
                )
            })?;

        if certs.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "no certificates found in certificate file",
            )
            .into());
        }

        Ok(certs)
    }

    /// Loads the private key from the key file.
    fn load_private_key(&self) -> Result<PrivateKeyDer<'static>, crate::Error> {
        let key_file = File::open(&self.key_path).map_err(|e| {
            std::io::Error::new(
                e.kind(),
                format!("failed to open private key file: {}", self.key_path.display()),
            )
        })?;

        let mut reader = BufReader::new(key_file);
        let key = rustls_pemfile::private_key(&mut reader)
            .map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("failed to parse private key: {e}"),
                )
            })?
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "no private key found in key file",
                )
            })?;

        Ok(key)
    }

    /// Builds a TLS acceptor from this configuration.
    pub fn build_acceptor(&self) -> Result<TlsAcceptor, crate::Error> {
        // Install the default crypto provider (ring) if not already installed
        let _ = rustls::crypto::ring::default_provider().install_default();

        let certs = self.load_certs()?;
        let key = self.load_private_key()?;

        let mut config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .map_err(|e| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("TLS error: {e}"))
            })?;

        // Set ALPN protocols to support HTTP/1.1 and HTTP/2
        config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

        Ok(TlsAcceptor::from(Arc::new(config)))
    }
}

/// Generates a self-signed certificate and private key for development use.
///
/// This function creates a self-signed certificate valid for localhost and 127.0.0.1.
/// The generated files are saved to the specified directory.
///
/// # Arguments
///
/// * `output_dir` - Directory where the certificate and key files will be saved
///
/// # Returns
///
/// Returns a tuple of (cert_path, key_path) on success
///
/// # Note
///
/// This is intended for development and testing only. For production, use proper
/// certificates from a trusted Certificate Authority.
pub fn generate_self_signed_cert(
    output_dir: impl AsRef<std::path::Path>,
) -> Result<(PathBuf, PathBuf), crate::Error> {
    use rcgen::{generate_simple_self_signed, CertifiedKey};

    // Install the default crypto provider (ring) if not already installed
    let _ = rustls::crypto::ring::default_provider().install_default();

    let subject_alt_names = vec!["localhost".to_string(), "127.0.0.1".to_string()];
    let CertifiedKey { cert, key_pair } =
        generate_simple_self_signed(subject_alt_names).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::Other, format!("cert generation error: {e}"))
        })?;

    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();

    let output_dir = output_dir.as_ref();
    std::fs::create_dir_all(output_dir)?;

    let cert_path = output_dir.join("cert.pem");
    let key_path = output_dir.join("key.pem");

    std::fs::write(&cert_path, cert_pem)?;
    std::fs::write(&key_path, key_pem)?;

    // Set secure permissions on the private key
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&key_path)?.permissions();
        perms.set_mode(0o600); // Read/write for owner only
        std::fs::set_permissions(&key_path, perms)?;
    }

    tracing::info!("Generated self-signed certificate at: {}", cert_path.display());
    tracing::info!("Generated private key at: {}", key_path.display());

    Ok((cert_path, key_path))
}

/// A TLS-enabled TCP stream.
pub type TlsStream = tokio_rustls::server::TlsStream<TcpStream>;

/// TLS connection acceptor that wraps a TCP listener.
pub struct TlsListener {
    listener: TcpListener,
    acceptor: TlsAcceptor,
    is_self_signed: bool,
}

impl TlsListener {
    /// Creates a new TLS listener.
    pub async fn bind(addr: SocketAddr, config: TlsConfig) -> Result<Self, crate::Error> {
        let listener = TcpListener::bind(addr).await?;
        let is_self_signed = config.is_self_signed;
        let acceptor = config.build_acceptor()?;
        Ok(Self { listener, acceptor, is_self_signed })
    }

    /// Returns the local address that this listener is bound to.
    pub fn local_addr(&self) -> Result<SocketAddr, std::io::Error> {
        self.listener.local_addr()
    }

    /// Accepts a new TLS connection.
    pub async fn accept(&self) -> Result<(TlsStream, SocketAddr), crate::Error> {
        let (stream, peer_addr) = self.listener.accept().await?;

        debug!(peer = %peer_addr, "Accepting TLS connection");

        match self.acceptor.accept(stream).await {
            Ok(tls_stream) => {
                debug!(peer = %peer_addr, "TLS handshake successful");
                Ok((tls_stream, peer_addr))
            }
            Err(e) => {
                // For self-signed certificates, certificate errors are expected when clients
                // don't explicitly trust them. Log at debug level instead of error.
                if self.is_self_signed
                    && (e.to_string().contains("CertificateUnknown")
                        || e.to_string().contains("UnknownCA")
                        || e.to_string().contains("certificate"))
                {
                    debug!(peer = %peer_addr, error = %e, "TLS handshake failed (expected with self-signed certificates)");
                } else {
                    error!(peer = %peer_addr, error = %e, "TLS handshake failed");
                }
                Err(e.into())
            }
        }
    }
}
