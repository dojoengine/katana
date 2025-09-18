use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use katana_core::backend::Backend;
use katana_executor::implementation::blockifier::BlockifierFactory;
use katana_primitives::Felt;
use serde::Deserialize;
use serde_json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpSocket, TcpStream};
use tracing::{error, info};

use crate::handlers;
use crate::handlers::HandlerError;

use crate::types::BlockId;

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
    pub addr: std::net::IpAddr,
    pub port: u16,
    pub timeout: Duration,
    pub share_rpc_port: bool,
}

impl Default for FeederGatewayConfig {
    fn default() -> Self {
        Self {
            addr: std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            port: DEFAULT_FEEDER_GATEWAY_PORT,
            timeout: DEFAULT_FEEDER_GATEWAY_TIMEOUT,
            share_rpc_port: false,
        }
    }
}

impl FeederGatewayConfig {
    /// Returns the [`SocketAddr`] for the feeder gateway server.
    pub fn socket_addr(&self) -> SocketAddr {
        SocketAddr::new(self.addr, self.port)
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

    /// Set the server address. Default is localhost.
    pub fn addr(mut self, addr: std::net::IpAddr) -> Self {
        self.config.addr = addr;
        self
    }

    /// Set the server port. Default is 5051.
    pub fn port(mut self, port: u16) -> Self {
        self.config.port = port;
        self
    }

    /// Set the request timeout. Default is 30 seconds.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.config.timeout = timeout;
        self
    }

    /// Enable sharing the RPC server's port instead of running independently.
    pub fn share_rpc_port(mut self, enable: bool) -> Self {
        self.config.share_rpc_port = enable;
        self
    }

    /// Start the feeder gateway server.
    ///
    /// If `share_rpc_port` is enabled, this will bind to the same address:port as the RPC server
    /// using SO_REUSEPORT, allowing both servers to coexist on the same endpoint.
    pub async fn start(self) -> Result<FeederGatewayServerHandle, Error> {
        self.start_server().await
    }

    /// Start the server, optionally with port reuse.
    async fn start_server(self) -> Result<FeederGatewayServerHandle, Error> {
        let addr = self.config.socket_addr();

        let listener = if self.config.share_rpc_port {
            // Create socket with SO_REUSEPORT for sharing with RPC server
            self.create_reuse_port_listener(addr).await?
        } else {
            // Standard bind
            TcpListener::bind(addr).await?
        };

        let actual_addr = listener.local_addr()?;

        let backend = self.backend.clone();
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();

        // Start HTTP server with proper request handling
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => {
                        info!(target: "feeder_gateway", "Feeder gateway server shutting down.");
                        break;
                    }
                    result = listener.accept() => {
                        match result {
                            Ok((stream, addr)) => {
                                let backend = backend.clone();
                                tokio::spawn(async move {
                                    if let Err(e) = handle_connection(stream, backend).await {
                                        error!("Failed to handle connection from {}: {}", addr, e);
                                    }
                                });
                            }
                            Err(e) => {
                                error!("Failed to accept connection: {}", e);
                            }
                        }
                    }
                }
            }
        });

        info!(target: "feeder_gateway", addr = %actual_addr, "Feeder gateway server started.");

        Ok(FeederGatewayServerHandle { addr: actual_addr, shutdown_tx: Some(shutdown_tx) })
    }

    /// Create a TcpListener with SO_REUSEPORT for sharing the same address:port
    async fn create_reuse_port_listener(&self, addr: SocketAddr) -> Result<TcpListener, Error> {
        let socket = if addr.is_ipv6() { TcpSocket::new_v6()? } else { TcpSocket::new_v4()? };
        let _ = socket.set_reuseport(true);

        // Enable SO_REUSEPORT for port sharing
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            let fd = socket.as_raw_fd();
            let optval: libc::c_int = 1;
            unsafe {
                if libc::setsockopt(
                    fd,
                    libc::SOL_SOCKET,
                    libc::SO_REUSEPORT,
                    &optval as *const _ as *const libc::c_void,
                    std::mem::size_of_val(&optval) as libc::socklen_t,
                ) < 0
                {
                    return Err(Error::Io(std::io::Error::last_os_error()));
                }
            }
        }

        #[cfg(windows)]
        {
            // Windows doesn't have SO_REUSEPORT, use SO_REUSEADDR instead
            use std::os::windows::io::AsRawSocket;
            use windows_sys::Win32::Networking::WinSock::{
                setsockopt, SOCKET_ERROR, SOL_SOCKET, SO_REUSEADDR,
            };

            let socket_raw = socket.as_raw_socket();
            let optval: i32 = 1;
            unsafe {
                if setsockopt(
                    socket_raw as _,
                    SOL_SOCKET,
                    SO_REUSEADDR,
                    &optval as *const _ as *const i8,
                    std::mem::size_of_val(&optval) as i32,
                ) == SOCKET_ERROR
                {
                    return Err(Error::Io(std::io::Error::last_os_error()));
                }
            }
        }

        socket.bind(addr)?;
        socket.listen(1024).map_err(Error::Io)
    }

    /// Returns the server configuration.
    pub fn get_config(&self) -> &FeederGatewayConfig {
        &self.config
    }
}

/// Handle incoming HTTP connection
async fn handle_connection(
    mut stream: TcpStream,
    backend: Arc<Backend<BlockifierFactory>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut buffer = [0; 4096];
    let bytes_read = stream.read(&mut buffer).await?;
    
    if bytes_read == 0 {
        return Ok(());
    }

    let request = String::from_utf8_lossy(&buffer[..bytes_read]);
    
    // Parse HTTP request line
    let lines: Vec<&str> = request.lines().collect();
    if lines.is_empty() {
        return send_error_response(&mut stream, 400, "Bad Request").await;
    }
    
    let request_line = lines[0];
    let parts: Vec<&str> = request_line.split_whitespace().collect();
    
    if parts.len() != 3 {
        return send_error_response(&mut stream, 400, "Bad Request").await;
    }
    
    let method = parts[0];
    let uri = parts[1];
    let _version = parts[2];
    
    // Only support GET requests
    if method != "GET" {
        return send_error_response(&mut stream, 405, "Method Not Allowed").await;
    }
    
    // Parse URI into path and query
    let (path, query) = if let Some(pos) = uri.find('?') {
        (&uri[..pos], &uri[pos + 1..])
    } else {
        (uri, "")
    };
    
    // Route to appropriate handler
    let response = match path {
        "/feeder_gateway/get_block" => {
            handle_get_block(backend, query).await
        }
        "/feeder_gateway/get_state_update" => {
            handle_get_state_update(backend, query).await
        }
        "/feeder_gateway/get_class_by_hash" => {
            handle_get_class_by_hash(backend, query).await
        }
        "/feeder_gateway/get_compiled_class_by_class_hash" => {
            handle_get_compiled_class_by_class_hash(backend, query).await
        }
        _ => {
            let error = HandlerError::InvalidParameter("Unknown endpoint".to_string());
            HttpResponse::error(error.status_code(), &error.to_json().to_string())
        }
    };
    
    // Send response
    stream.write_all(response.to_string().as_bytes()).await?;
    stream.flush().await?;
    
    Ok(())
}

/// Handle /feeder_gateway/get_block endpoint
async fn handle_get_block(
    backend: Arc<Backend<BlockifierFactory>>,
    query: &str,
) -> HttpResponse {
    let params = handlers::parse_query_params(query);
    
    let block_id = match handlers::parse_block_id(&params) {
        Ok(id) => id,
        Err(e) => return HttpResponse::error(e.status_code(), &e.to_json().to_string()),
    };
    
    match handlers::get_block(backend, block_id).await {
        Ok(_block) => {
            // TODO: Serialize Block properly when all nested types support Serialize
            let json = r#"{"error": "Block serialization not yet implemented"}"#;
            HttpResponse::error(501, json)
        }
        Err(e) => HttpResponse::error(e.status_code(), &e.to_json().to_string()),
    }
}

/// Handle /feeder_gateway/get_state_update endpoint
async fn handle_get_state_update(
    backend: Arc<Backend<BlockifierFactory>>,
    query: &str,
) -> HttpResponse {
    let params = handlers::parse_query_params(query);
    
    let block_id = match handlers::parse_block_id(&params) {
        Ok(id) => id,
        Err(e) => return HttpResponse::error(e.status_code(), &e.to_json().to_string()),
    };
    
    let include_block = handlers::should_include_block(&params);
    
    match handlers::get_state_update(backend, block_id, include_block).await {
        Ok(result) => {
            let json = result.to_string();
            HttpResponse::ok(&json)
        }
        Err(e) => HttpResponse::error(e.status_code(), &e.to_json().to_string()),
    }
}

/// Handle /feeder_gateway/get_class_by_hash endpoint
async fn handle_get_class_by_hash(
    backend: Arc<Backend<BlockifierFactory>>,
    query: &str,
) -> HttpResponse {
    let params = handlers::parse_query_params(query);
    
    let class_hash = match handlers::parse_class_hash(&params) {
        Ok(hash) => hash,
        Err(e) => return HttpResponse::error(e.status_code(), &e.to_json().to_string()),
    };
    
    let block_id = match handlers::parse_block_id(&params) {
        Ok(id) => id,
        Err(e) => return HttpResponse::error(e.status_code(), &e.to_json().to_string()),
    };
    
    match handlers::get_class_by_hash(backend, class_hash, block_id).await {
        Ok(_class) => {
            // TODO: Serialize ContractClass properly  
            let json = r#"{"error": "ContractClass serialization not yet implemented"}"#;
            HttpResponse::error(501, json)
        }
        Err(e) => HttpResponse::error(e.status_code(), &e.to_json().to_string()),
    }
}

/// Handle /feeder_gateway/get_compiled_class_by_class_hash endpoint
async fn handle_get_compiled_class_by_class_hash(
    backend: Arc<Backend<BlockifierFactory>>,
    query: &str,
) -> HttpResponse {
    let params = handlers::parse_query_params(query);
    
    let class_hash = match handlers::parse_class_hash(&params) {
        Ok(hash) => hash,
        Err(e) => return HttpResponse::error(e.status_code(), &e.to_json().to_string()),
    };
    
    let block_id = match handlers::parse_block_id(&params) {
        Ok(id) => id,
        Err(e) => return HttpResponse::error(e.status_code(), &e.to_json().to_string()),
    };
    
    match handlers::get_compiled_class_by_class_hash(backend, class_hash, block_id).await {
        Ok(_class) => {
            // TODO: Serialize CasmContractClass properly
            let json = r#"{"error": "CasmContractClass serialization not yet implemented"}"#;
            HttpResponse::error(501, json)
        }
        Err(e) => HttpResponse::error(e.status_code(), &e.to_json().to_string()),
    }
}

/// Send error response
async fn send_error_response(
    stream: &mut TcpStream,
    status_code: u16,
    message: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let response = HttpResponse::error(status_code, message);
    stream.write_all(response.to_string().as_bytes()).await?;
    stream.flush().await?;
    Ok(())
}

/// Simple HTTP response builder
struct HttpResponse {
    status_code: u16,
    status_text: &'static str,
    body: String,
}

impl HttpResponse {
    fn ok(body: &str) -> Self {
        Self {
            status_code: 200,
            status_text: "OK",
            body: body.to_string(),
        }
    }
    
    fn error(status_code: u16, body: &str) -> Self {
        let status_text = match status_code {
            400 => "Bad Request",
            404 => "Not Found", 
            405 => "Method Not Allowed",
            500 => "Internal Server Error",
            501 => "Not Implemented",
            _ => "Error",
        };
        
        Self {
            status_code,
            status_text,
            body: body.to_string(),
        }
    }
}

impl std::fmt::Display for HttpResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\n\r\n{}",
            self.status_code,
            self.status_text,
            self.body.len(),
            self.body
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

// Parameter types for endpoint parsing
#[derive(Debug, Deserialize)]
struct BlockParams {
    #[serde(rename = "blockHash")]
    block_hash: Option<String>,
    #[serde(rename = "blockNumber")]
    block_number: Option<u64>,
}

impl BlockParams {
    fn to_block_id(&self) -> Result<BlockId, &'static str> {
        match (&self.block_hash, self.block_number) {
            (Some(hash_str), None) => {
                let hash = Felt::from_hex(hash_str).map_err(|_| "Invalid block hash")?;
                Ok(BlockId::Hash(hash))
            }
            (None, Some(number)) => Ok(BlockId::Number(number)),
            (None, None) => Ok(BlockId::Latest),
            (Some(_), Some(_)) => Err("Cannot specify both block hash and number"),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ClassParams {
    #[serde(rename = "classHash")]
    class_hash: String,
    #[serde(flatten)]
    block: BlockParams,
}
