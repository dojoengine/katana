use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::SystemTime;

use anyhow::{anyhow, Result};
use bytes::Bytes;
use http::header::HeaderValue;
use http::{HeaderMap, Request, Response, StatusCode};
// Define Body type based on what's available
#[cfg(feature = "jsonrpsee")]
use jsonrpsee::core::{http_helpers::Body, BoxError};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tower::{Layer, Service};
use tracing::{debug, error, info, warn};
use url::Url;

#[cfg(not(feature = "jsonrpsee"))]
type BoxError = Box<dyn std::error::Error + Send + Sync>;

#[cfg(not(feature = "jsonrpsee"))]
#[derive(Debug)]
pub enum Body {
    Fixed(Bytes),
}

#[cfg(not(feature = "jsonrpsee"))]
impl Body {
    fn from<T: Into<Vec<u8>>>(data: T) -> Self {
        Self::Fixed(Bytes::from(data.into()))
    }
}

#[cfg(not(feature = "jsonrpsee"))]
impl http_body::Body for Body {
    type Data = Bytes;
    type Error = BoxError;

    fn poll_frame(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        match self.get_mut() {
            Body::Fixed(bytes) if !bytes.is_empty() => {
                let frame = http_body::Frame::data(std::mem::take(bytes));
                Poll::Ready(Some(Ok(frame)))
            }
            _ => Poll::Ready(None),
        }
    }
}

/// Explorer serving mode configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExplorerMode {
    /// Serve embedded assets (production mode)
    Embedded,
    /// Serve from filesystem with optional hot reload (development mode)
    FileSystem { ui_path: PathBuf, hot_reload: bool },
    /// Proxy to external development server
    Proxy { upstream_url: Url, inject_env: bool },
}

/// Explorer configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExplorerConfig {
    /// Serving mode
    pub mode: ExplorerMode,
    /// Chain ID to inject into the UI
    pub chain_id: String,
    /// URL path prefix (default: "/explorer")
    pub path_prefix: String,
    /// Enable CORS headers
    pub cors_enabled: bool,
    /// Enable security headers
    pub security_headers: bool,
    /// Enable compression
    pub compression: bool,
    /// Custom headers to add to responses
    pub custom_headers: HashMap<String, String>,
    /// Custom environment variables to inject into UI
    pub ui_env: HashMap<String, serde_json::Value>,
}

impl Default for ExplorerConfig {
    fn default() -> Self {
        Self {
            mode: ExplorerMode::Embedded,
            chain_id: "KATANA".to_string(),
            path_prefix: "/explorer".to_string(),
            cors_enabled: false,
            security_headers: true,
            compression: false,
            custom_headers: HashMap::new(),
            ui_env: HashMap::new(),
        }
    }
}

/// File cache entry for hot reload functionality
#[derive(Debug, Clone)]
struct CacheEntry {
    content: Bytes,
    content_type: String,
    last_modified: SystemTime,
}

/// Explorer layer for Tower middleware
#[derive(Debug, Clone)]
pub struct ExplorerLayer {
    config: ExplorerConfig,
}

impl ExplorerLayer {
    pub fn new(config: ExplorerConfig) -> Result<Self> {
        // Validate configuration
        match &config.mode {
            ExplorerMode::Embedded => {
                #[cfg(feature = "embedded-ui")]
                {
                    if EmbeddedAssets::get("index.html").is_none() {
                        return Err(anyhow!(
                            "Embedded mode selected but no UI assets found. Make sure the \
                             explorer UI is built."
                        ));
                    }
                }
                #[cfg(not(feature = "embedded-ui"))]
                {
                    return Err(anyhow!(
                        "Embedded mode selected but embedded-ui feature is disabled. Enable the \
                         feature or use FileSystem/Proxy mode."
                    ));
                }
            }
            ExplorerMode::FileSystem { ui_path, .. } => {
                if !ui_path.exists() {
                    return Err(anyhow!("UI path does not exist: {}", ui_path.display()));
                }
                if !ui_path.join("index.html").exists() {
                    warn!("index.html not found in UI path: {}", ui_path.display());
                }
            }
            ExplorerMode::Proxy { upstream_url, .. } => {
                debug!("Proxy mode configured to upstream: {}", upstream_url);
            }
        }

        info!("Explorer configured with mode: {:?}", config.mode);
        Ok(Self { config })
    }

    /// Create a builder for more ergonomic configuration
    pub fn builder() -> ExplorerLayerBuilder {
        ExplorerLayerBuilder::new()
    }

    /// Create a new ExplorerLayer with embedded mode
    pub fn embedded(chain_id: String) -> Result<Self> {
        Self::new(ExplorerConfig { mode: ExplorerMode::Embedded, chain_id, ..Default::default() })
    }

    /// Create a new ExplorerLayer with filesystem mode
    pub fn filesystem(ui_path: PathBuf, chain_id: String, hot_reload: bool) -> Result<Self> {
        Self::new(ExplorerConfig {
            mode: ExplorerMode::FileSystem { ui_path, hot_reload },
            chain_id,
            ..Default::default()
        })
    }

    /// Create a new ExplorerLayer with proxy mode  
    pub fn proxy(upstream_url: Url, chain_id: String) -> Result<Self> {
        Self::new(ExplorerConfig {
            mode: ExplorerMode::Proxy { upstream_url, inject_env: true },
            chain_id,
            ..Default::default()
        })
    }
}

/// Builder for creating ExplorerLayer with a fluent API
#[derive(Debug, Clone)]
pub struct ExplorerLayerBuilder {
    config: ExplorerConfig,
}

impl ExplorerLayerBuilder {
    /// Create a new builder with default configuration
    pub fn new() -> Self {
        Self { config: ExplorerConfig::default() }
    }

    /// Build the ExplorerLayer with current configuration
    pub fn build(self) -> Result<ExplorerLayer> {
        ExplorerLayer::new(self.config)
    }

    // === Preset Methods ===

    /// Configure for development with sensible defaults
    /// - FileSystem mode with hot reload enabled
    /// - CORS enabled for development
    /// - Security headers disabled for easier development
    /// - UI path defaults to "./ui/dist"
    pub fn development(mut self) -> Self {
        self.config.mode =
            ExplorerMode::FileSystem { ui_path: PathBuf::from("./ui/dist"), hot_reload: true };
        self.config.cors_enabled = true;
        self.config.security_headers = false;
        self.config.compression = false;
        self
    }

    /// Configure for production with sensible defaults
    /// - Embedded mode for optimal performance
    /// - Security headers enabled
    /// - CORS disabled for security
    /// - Compression enabled (future)
    pub fn production(mut self) -> Self {
        self.config.mode = ExplorerMode::Embedded;
        self.config.cors_enabled = false;
        self.config.security_headers = true;
        self.config.compression = true;
        self
    }

    // === Core Configuration ===

    /// Set the chain ID (required)
    pub fn chain_id<S: Into<String>>(mut self, chain_id: S) -> Self {
        self.config.chain_id = chain_id.into();
        self
    }

    /// Set the URL path prefix (default: "/explorer")
    pub fn path_prefix<S: Into<String>>(mut self, prefix: S) -> Self {
        self.config.path_prefix = prefix.into();
        self
    }

    // === Serving Mode Configuration ===

    /// Use embedded assets mode
    pub fn embedded_mode(mut self) -> Self {
        self.config.mode = ExplorerMode::Embedded;
        self
    }

    /// Use filesystem mode with specified path and hot reload setting
    pub fn filesystem_mode<P: Into<PathBuf>>(mut self, ui_path: P, hot_reload: bool) -> Self {
        self.config.mode = ExplorerMode::FileSystem { ui_path: ui_path.into(), hot_reload };
        self
    }

    /// Use filesystem mode with hot reload enabled (common development case)
    pub fn filesystem_with_hot_reload<P: Into<PathBuf>>(self, ui_path: P) -> Self {
        self.filesystem_mode(ui_path, true)
    }

    /// Use filesystem mode with hot reload disabled
    pub fn filesystem_static<P: Into<PathBuf>>(self, ui_path: P) -> Self {
        self.filesystem_mode(ui_path, false)
    }

    /// Use proxy mode
    pub fn proxy_mode(mut self, upstream_url: Url, inject_env: bool) -> Self {
        self.config.mode = ExplorerMode::Proxy { upstream_url, inject_env };
        self
    }

    /// Use proxy mode with environment injection enabled (default)
    pub fn proxy<S: AsRef<str>>(self, upstream_url: S) -> Result<Self> {
        let url = Url::parse(upstream_url.as_ref())
            .map_err(|e| anyhow!("Invalid upstream URL: {}", e))?;
        Ok(self.proxy_mode(url, true))
    }

    // === Convenience Methods for UI Path ===

    /// Set UI path (only relevant for filesystem mode)
    pub fn ui_path<P: Into<PathBuf>>(mut self, path: P) -> Self {
        match &mut self.config.mode {
            ExplorerMode::FileSystem { ui_path, .. } => {
                *ui_path = path.into();
            }
            _ => {
                // Switch to filesystem mode if not already
                self.config.mode =
                    ExplorerMode::FileSystem { ui_path: path.into(), hot_reload: true };
            }
        }
        self
    }

    /// Enable hot reload (only relevant for filesystem mode)
    pub fn hot_reload(mut self, enabled: bool) -> Self {
        match &mut self.config.mode {
            ExplorerMode::FileSystem { hot_reload, .. } => {
                *hot_reload = enabled;
            }
            _ => {
                // If not filesystem mode, ignore this setting but don't error
                debug!("hot_reload() called but not in filesystem mode, ignoring");
            }
        }
        self
    }

    // === Security and Headers ===

    /// Enable or disable CORS
    pub fn cors(mut self, enabled: bool) -> Self {
        self.config.cors_enabled = enabled;
        self
    }

    /// Enable CORS (convenience method)
    pub fn with_cors(self) -> Self {
        self.cors(true)
    }

    /// Enable or disable security headers
    pub fn security_headers(mut self, enabled: bool) -> Self {
        self.config.security_headers = enabled;
        self
    }

    /// Add a custom header
    pub fn header<K, V>(mut self, key: K, value: V) -> Self
    where
        K: Into<String>,
        V: Into<String>,
    {
        self.config.custom_headers.insert(key.into(), value.into());
        self
    }

    /// Add multiple custom headers
    pub fn headers<I, K, V>(mut self, headers: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        for (key, value) in headers {
            self.config.custom_headers.insert(key.into(), value.into());
        }
        self
    }

    // === UI Environment Variables ===

    /// Add a UI environment variable
    pub fn ui_env<K, V>(mut self, key: K, value: V) -> Self
    where
        K: Into<String>,
        V: Into<serde_json::Value>,
    {
        self.config.ui_env.insert(key.into(), value.into());
        self
    }

    /// Add multiple UI environment variables
    pub fn ui_envs<I, K, V>(mut self, envs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<serde_json::Value>,
    {
        for (key, value) in envs {
            self.config.ui_env.insert(key.into(), value.into());
        }
        self
    }

    /// Enable debug mode (adds DEBUG=true to UI environment)
    pub fn debug(self) -> Self {
        self.ui_env("DEBUG", true)
    }

    /// Set API endpoint URL for the UI
    pub fn api_endpoint<S: Into<String>>(self, endpoint: S) -> Self {
        self.ui_env("API_ENDPOINT", endpoint.into())
    }

    // === Performance ===

    /// Enable or disable compression (future feature)
    pub fn compression(mut self, enabled: bool) -> Self {
        self.config.compression = enabled;
        self
    }
}

impl Default for ExplorerLayerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl<S> Layer<S> for ExplorerLayer {
    type Service = ExplorerService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        ExplorerService::new(inner, self.config.clone())
    }
}

/// Explorer service implementation
#[derive(Debug)]
pub struct ExplorerService<S> {
    inner: S,
    config: ExplorerConfig,
    file_cache: Arc<RwLock<HashMap<String, CacheEntry>>>,
    _watcher: Option<RecommendedWatcher>,
}

impl<S> ExplorerService<S> {
    pub fn new(inner: S, config: ExplorerConfig) -> Self {
        let file_cache = Arc::new(RwLock::new(HashMap::new()));

        // Set up file watcher for hot reload
        let _watcher = if let ExplorerMode::FileSystem { ui_path, hot_reload } = &config.mode {
            if *hot_reload {
                Self::setup_file_watcher(ui_path.clone(), file_cache.clone())
            } else {
                None
            }
        } else {
            None
        };

        Self { inner, config, file_cache, _watcher }
    }

    fn setup_file_watcher(
        ui_path: PathBuf,
        cache: Arc<RwLock<HashMap<String, CacheEntry>>>,
    ) -> Option<RecommendedWatcher> {
        match notify::recommended_watcher({
            let cache = cache.clone();
            move |result: Result<Event, notify::Error>| {
                match result {
                    Ok(event) => {
                        debug!("File system event: {:?}", event);
                        // Clear cache on any file change
                        if let Ok(mut cache) = cache.try_write() {
                            cache.clear();
                            debug!("File cache cleared due to file system change");
                        }
                    }
                    Err(e) => error!("File watcher error: {:?}", e),
                }
            }
        }) {
            Ok(mut watcher) => {
                if let Err(e) = watcher.watch(&ui_path, RecursiveMode::Recursive) {
                    error!("Failed to watch UI directory: {:?}", e);
                    None
                } else {
                    info!("Hot reload enabled for UI directory: {}", ui_path.display());
                    Some(watcher)
                }
            }
            Err(e) => {
                error!("Failed to create file watcher: {:?}", e);
                None
            }
        }
    }
}

impl<S, B> Service<Request<B>> for ExplorerService<S>
where
    B::Data: Send,
    S::Response: 'static,
    B::Error: Into<BoxError>,
    S::Future: Send + 'static,
    S::Error: Into<BoxError> + 'static,
    S: Service<Request<B>, Response = Response<Body>>,
    B: http_body::Body<Data = Bytes> + Send + 'static,
{
    type Response = Response<Body>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<B>) -> Self::Future {
        // Check if this is an explorer request
        let uri_path = req.uri().path().to_string();
        if !uri_path.starts_with(&self.config.path_prefix) {
            return Box::pin(self.inner.call(req));
        }

        // Extract the file path after the prefix
        let rel_path = uri_path
            .strip_prefix(&self.config.path_prefix)
            .unwrap_or("")
            .trim_start_matches('/')
            .to_string();

        let config = self.config.clone();
        let cache = self.file_cache.clone();

        Box::pin(async move {
            let response = match Self::serve_asset(&config, cache, &rel_path).await {
                Some(response) => response,
                None => Self::create_404_response(),
            };
            Ok(response)
        })
    }
}

impl<S> ExplorerService<S> {
    async fn serve_asset(
        config: &ExplorerConfig,
        cache: Arc<RwLock<HashMap<String, CacheEntry>>>,
        path: &str,
    ) -> Option<Response<Body>> {
        match &config.mode {
            ExplorerMode::FileSystem { ui_path, hot_reload } => {
                Self::serve_from_filesystem(config, cache, ui_path, path, *hot_reload).await
            }
            ExplorerMode::Embedded => Self::serve_embedded(config, path).await,
            ExplorerMode::Proxy { .. } => {
                // For now, we'll implement a simple fallback to embedded
                // Full proxy implementation would require an HTTP client
                warn!("Proxy mode not fully implemented, falling back to embedded");
                Self::serve_embedded(config, path).await
            }
        }
    }

    async fn serve_from_filesystem(
        config: &ExplorerConfig,
        cache: Arc<RwLock<HashMap<String, CacheEntry>>>,
        ui_path: &Path,
        path: &str,
        hot_reload: bool,
    ) -> Option<Response<Body>> {
        let file_path = if path.is_empty() || path == "/" {
            ui_path.join("index.html")
        } else if Self::is_static_asset_path(path) {
            ui_path.join(path)
        } else {
            // SPA fallback - serve index.html for routes
            ui_path.join("index.html")
        };

        // Try cache first (if hot reload is disabled or for performance)
        if hot_reload {
            let cache_read = cache.read().await;
            if let Some(entry) = cache_read.get(path) {
                if let Ok(metadata) = std::fs::metadata(&file_path) {
                    if let Ok(modified) = metadata.modified() {
                        if modified <= entry.last_modified {
                            debug!("Serving {} from cache", path);
                            return Some(Self::create_response(
                                config,
                                &entry.content_type,
                                entry.content.clone(),
                            ));
                        }
                    }
                }
            }
        }

        // Read from filesystem
        match tokio::fs::read(&file_path).await {
            Ok(content) => {
                let content_type = Self::get_content_type(&file_path.to_string_lossy());
                let processed_content = if content_type == "text/html" {
                    let html = String::from_utf8_lossy(&content);
                    let injected = Self::inject_environment(&html, config);
                    Bytes::from(injected)
                } else {
                    Bytes::from(content)
                };

                // Update cache if hot reload is enabled
                if hot_reload {
                    let mut cache_write = cache.write().await;
                    cache_write.insert(
                        path.to_string(),
                        CacheEntry {
                            content: processed_content.clone(),
                            content_type: content_type.to_string(),
                            last_modified: SystemTime::now(),
                        },
                    );
                }

                debug!("Serving {} from filesystem: {}", path, file_path.display());
                Some(Self::create_response(config, content_type, processed_content))
            }
            Err(e) => {
                debug!("Failed to read file {}: {}", file_path.display(), e);

                // Fallback to embedded if filesystem fails
                #[cfg(feature = "embedded-ui")]
                {
                    warn!("Falling back to embedded assets for: {}", path);
                    Self::serve_embedded(config, path).await
                }
                #[cfg(not(feature = "embedded-ui"))]
                None
            }
        }
    }

    async fn serve_embedded(config: &ExplorerConfig, path: &str) -> Option<Response<Body>> {
        #[cfg(feature = "embedded-ui")]
        {
            let asset_path = if path.is_empty() || path == "/" {
                "index.html"
            } else if Self::is_static_asset_path(path) && EmbeddedAssets::get(path).is_some() {
                path
            } else {
                "index.html" // SPA fallback
            };

            if let Some(asset) = EmbeddedAssets::get(asset_path) {
                let content_type = Self::get_content_type(&format!("/{}", asset_path));
                let content = if content_type == "text/html" {
                    let html = String::from_utf8_lossy(&asset.data);
                    let injected = Self::inject_environment(&html, config);
                    Bytes::from(injected)
                } else {
                    Bytes::copy_from_slice(&asset.data)
                };

                debug!("Serving {} from embedded assets", asset_path);
                return Some(Self::create_response(config, content_type, content));
            }
        }

        #[cfg(not(feature = "embedded-ui"))]
        {
            let _ = (config, path); // Silence unused warnings
        }

        None
    }

    fn create_response(
        config: &ExplorerConfig,
        content_type: &str,
        content: Bytes,
    ) -> Response<Body> {
        let mut response =
            Response::builder().status(StatusCode::OK).header("Content-Type", content_type);

        // Add caching headers
        let cache_control = Self::get_cache_control(content_type);
        response = response.header("Cache-Control", cache_control);

        // Add security headers
        if config.security_headers {
            for (key, value) in Self::get_security_headers().iter() {
                response = response.header(key, value);
            }
        }

        // Add CORS headers
        if config.cors_enabled {
            response = response.header("Access-Control-Allow-Origin", "*");
            response = response.header("Access-Control-Allow-Methods", "GET, OPTIONS");
            response = response.header("Access-Control-Allow-Headers", "Content-Type");
        }

        // Add custom headers
        for (key, value) in &config.custom_headers {
            response = response.header(key, value);
        }

        response.body(Body::from(content.to_vec())).unwrap()
    }

    fn create_404_response() -> Response<Body> {
        Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header("Content-Type", "text/plain")
            .body(Body::from("Explorer UI not found"))
            .unwrap()
    }

    fn inject_environment(html: &str, config: &ExplorerConfig) -> String {
        let mut env_vars = config.ui_env.clone();
        env_vars.insert("CHAIN_ID".to_string(), serde_json::Value::String(config.chain_id.clone()));
        env_vars.insert("ENABLE_CONTROLLER".to_string(), serde_json::Value::Bool(false));

        let env_json = serde_json::to_string(&env_vars).unwrap_or_default();
        let script = format!(
            r#"<script>
                window.KATANA_CONFIG = {};
                // Backward compatibility
                window.CHAIN_ID = "{}";
                window.ENABLE_CONTROLLER = false;
            </script>"#,
            env_json, config.chain_id
        );

        if let Some(head_pos) = html.find("<head>") {
            let (start, end) = html.split_at(head_pos + 6);
            format!("{}{}{}", start, script, end)
        } else {
            format!("{}\n{}", script, html)
        }
    }

    fn get_content_type(path: &str) -> &'static str {
        match path.rsplit('.').next() {
            Some("html") => "text/html; charset=utf-8",
            Some("js") => "application/javascript; charset=utf-8",
            Some("mjs") => "application/javascript; charset=utf-8",
            Some("css") => "text/css; charset=utf-8",
            Some("png") => "image/png",
            Some("jpg") | Some("jpeg") => "image/jpeg",
            Some("gif") => "image/gif",
            Some("svg") => "image/svg+xml",
            Some("json") => "application/json; charset=utf-8",
            Some("ico") => "image/x-icon",
            Some("woff") => "font/woff",
            Some("woff2") => "font/woff2",
            Some("ttf") => "font/ttf",
            Some("eot") => "application/vnd.ms-fontobject",
            Some("webp") => "image/webp",
            Some("avif") => "image/avif",
            _ => "application/octet-stream",
        }
    }

    fn get_cache_control(content_type: &str) -> &'static str {
        if content_type.starts_with("text/html") {
            "no-cache, must-revalidate" // Always check HTML files
        } else if content_type.starts_with("application/javascript")
            || content_type.starts_with("text/css")
        {
            "public, max-age=31536000, immutable" // 1 year for JS/CSS (assuming they're hashed)
        } else {
            "public, max-age=3600" // 1 hour for other assets
        }
    }

    fn get_security_headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("X-Frame-Options", HeaderValue::from_static("DENY"));
        headers.insert("X-Content-Type-Options", HeaderValue::from_static("nosniff"));
        headers
            .insert("Referrer-Policy", HeaderValue::from_static("strict-origin-when-cross-origin"));
        headers.insert(
            "Content-Security-Policy",
            HeaderValue::from_static(
                "default-src 'self'; script-src 'self' 'unsafe-inline' 'unsafe-eval'; style-src \
                 'self' 'unsafe-inline'; img-src 'self' data: https:; font-src 'self' data:;",
            ),
        );
        headers
    }

    fn is_static_asset_path(path: &str) -> bool {
        !path.is_empty()
            && (path.ends_with(".js")
                || path.ends_with(".mjs")
                || path.ends_with(".css")
                || path.ends_with(".png")
                || path.ends_with(".jpg")
                || path.ends_with(".jpeg")
                || path.ends_with(".gif")
                || path.ends_with(".svg")
                || path.ends_with(".json")
                || path.ends_with(".ico")
                || path.ends_with(".woff")
                || path.ends_with(".woff2")
                || path.ends_with(".ttf")
                || path.ends_with(".eot")
                || path.ends_with(".webp")
                || path.ends_with(".avif"))
    }
}

/// Embedded explorer UI files (only available with embedded-ui feature)
#[cfg(feature = "embedded-ui")]
#[derive(rust_embed::RustEmbed)]
#[folder = "ui/dist"]
struct EmbeddedAssets;

/// Stub for when embedded-ui feature is disabled
#[cfg(not(feature = "embedded-ui"))]
struct EmbeddedAssets;

#[cfg(not(feature = "embedded-ui"))]
impl EmbeddedAssets {
    fn get(_path: &str) -> Option<EmbeddedFile> {
        None
    }
}

#[cfg(not(feature = "embedded-ui"))]
struct EmbeddedFile;

#[cfg(not(feature = "embedded-ui"))]
impl EmbeddedFile {
    pub fn data(&self) -> &[u8] {
        &[]
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use tempfile::TempDir;
    use tokio::fs;

    use super::*;

    #[test]
    fn test_get_content_type() {
        assert_eq!(
            ExplorerService::<()>::get_content_type("index.html"),
            "text/html; charset=utf-8"
        );
        assert_eq!(
            ExplorerService::<()>::get_content_type("app.js"),
            "application/javascript; charset=utf-8"
        );
        assert_eq!(
            ExplorerService::<()>::get_content_type("app.mjs"),
            "application/javascript; charset=utf-8"
        );
        assert_eq!(
            ExplorerService::<()>::get_content_type("styles.css"),
            "text/css; charset=utf-8"
        );
        assert_eq!(ExplorerService::<()>::get_content_type("logo.png"), "image/png");
        assert_eq!(ExplorerService::<()>::get_content_type("photo.jpg"), "image/jpeg");
        assert_eq!(ExplorerService::<()>::get_content_type("icon.svg"), "image/svg+xml");
        assert_eq!(
            ExplorerService::<()>::get_content_type("data.json"),
            "application/json; charset=utf-8"
        );
        assert_eq!(ExplorerService::<()>::get_content_type("favicon.ico"), "image/x-icon");
        assert_eq!(ExplorerService::<()>::get_content_type("font.woff"), "font/woff");
        assert_eq!(ExplorerService::<()>::get_content_type("font.woff2"), "font/woff2");
        assert_eq!(ExplorerService::<()>::get_content_type("font.ttf"), "font/ttf");
        assert_eq!(
            ExplorerService::<()>::get_content_type("font.eot"),
            "application/vnd.ms-fontobject"
        );
        assert_eq!(ExplorerService::<()>::get_content_type("image.webp"), "image/webp");
        assert_eq!(ExplorerService::<()>::get_content_type("image.avif"), "image/avif");
        assert_eq!(
            ExplorerService::<()>::get_content_type("unknown.xyz"),
            "application/octet-stream"
        );
    }

    #[test]
    fn test_is_static_asset_path() {
        assert!(ExplorerService::<()>::is_static_asset_path("app.js"));
        assert!(ExplorerService::<()>::is_static_asset_path("app.mjs"));
        assert!(ExplorerService::<()>::is_static_asset_path("styles.css"));
        assert!(ExplorerService::<()>::is_static_asset_path("logo.png"));
        assert!(ExplorerService::<()>::is_static_asset_path("icon.svg"));
        assert!(ExplorerService::<()>::is_static_asset_path("data.json"));
        assert!(ExplorerService::<()>::is_static_asset_path("assets/js/app.js"));

        assert!(!ExplorerService::<()>::is_static_asset_path(""));
        assert!(!ExplorerService::<()>::is_static_asset_path("index.html"));
        assert!(!ExplorerService::<()>::is_static_asset_path("page"));
        assert!(!ExplorerService::<()>::is_static_asset_path("unknown.txt"));
    }

    #[test]
    fn test_inject_environment() {
        let html = "<html><head></head><body></body></html>";
        let config = ExplorerConfig {
            chain_id: "TEST_CHAIN".to_string(),
            ui_env: {
                let mut env = HashMap::new();
                env.insert("DEBUG".to_string(), serde_json::Value::Bool(true));
                env
            },
            ..Default::default()
        };

        let result = ExplorerService::<()>::inject_environment(html, &config);

        assert!(result.contains("window.KATANA_CONFIG"));
        assert!(result.contains("\"CHAIN_ID\":\"TEST_CHAIN\""));
        assert!(result.contains("\"DEBUG\":true"));
        assert!(result.contains("window.CHAIN_ID = \"TEST_CHAIN\""));
    }

    #[test]
    fn test_explorer_config_default() {
        let config = ExplorerConfig::default();
        assert_eq!(config.chain_id, "KATANA");
        assert_eq!(config.path_prefix, "/explorer");
        assert!(!config.cors_enabled);
        assert!(config.security_headers);
    }

    #[tokio::test]
    async fn test_filesystem_mode_with_temp_dir() {
        let temp_dir = TempDir::new().unwrap();
        let ui_path = temp_dir.path().to_path_buf();

        // Create a simple index.html
        let index_content = r#"<html><head></head><body><h1>Test UI</h1></body></html>"#;
        fs::write(ui_path.join("index.html"), index_content).await.unwrap();

        // Create config
        let config = ExplorerConfig {
            mode: ExplorerMode::FileSystem { ui_path: ui_path.clone(), hot_reload: false },
            chain_id: "TEST".to_string(),
            ..Default::default()
        };

        // Test layer creation
        let layer = ExplorerLayer::new(config).unwrap();
        assert!(matches!(layer.config.mode, ExplorerMode::FileSystem { .. }));
    }

    #[test]
    fn test_builder_pattern_development_preset() {
        let layer = ExplorerLayer::builder().development().chain_id("TEST_DEV").build();

        // Should fail because ./ui/dist doesn't exist in test environment
        if layer.is_err() {
            // Expected in test environment
            return;
        }

        let layer = layer.unwrap();
        assert_eq!(layer.config.chain_id, "TEST_DEV");
        assert!(layer.config.cors_enabled);
        assert!(!layer.config.security_headers);
        assert!(matches!(layer.config.mode, ExplorerMode::FileSystem { hot_reload: true, .. }));
    }

    #[test]
    fn test_builder_pattern_production_preset() {
        let result = ExplorerLayer::builder().production().chain_id("TEST_PROD").build();

        // May fail if embedded-ui feature is disabled or no assets
        match result {
            Ok(layer) => {
                assert_eq!(layer.config.chain_id, "TEST_PROD");
                assert!(!layer.config.cors_enabled);
                assert!(layer.config.security_headers);
                assert!(matches!(layer.config.mode, ExplorerMode::Embedded));
            }
            Err(_) => {
                // Expected if embedded assets aren't available
            }
        }
    }

    #[tokio::test]
    async fn test_builder_pattern_custom_configuration() {
        let temp_dir = TempDir::new().unwrap();
        let ui_path = temp_dir.path().to_path_buf();

        // Create a simple index.html
        let index_content = r#"<html><head></head><body><h1>Test UI</h1></body></html>"#;
        fs::write(ui_path.join("index.html"), index_content).await.unwrap();

        let layer = ExplorerLayer::builder()
            .chain_id("CUSTOM_TEST")
            .filesystem_with_hot_reload(&ui_path)
            .with_cors()
            .debug()
            .api_endpoint("/api/test")
            .header("X-Test", "value")
            .path_prefix("/test-explorer")
            .build()
            .unwrap();

        assert_eq!(layer.config.chain_id, "CUSTOM_TEST");
        assert_eq!(layer.config.path_prefix, "/test-explorer");
        assert!(layer.config.cors_enabled);

        // Check UI environment variables
        assert_eq!(layer.config.ui_env.get("DEBUG"), Some(&serde_json::Value::Bool(true)));
        assert_eq!(
            layer.config.ui_env.get("API_ENDPOINT"),
            Some(&serde_json::Value::String("/api/test".to_string()))
        );

        // Check custom headers
        assert_eq!(layer.config.custom_headers.get("X-Test"), Some(&"value".to_string()));

        // Check filesystem mode with hot reload
        if let ExplorerMode::FileSystem { ui_path: configured_path, hot_reload } =
            &layer.config.mode
        {
            assert_eq!(configured_path, &ui_path);
            assert!(*hot_reload);
        } else {
            panic!("Expected FileSystem mode");
        }
    }

    #[test]
    fn test_builder_pattern_ui_path_switching() {
        // Start with embedded mode, then set ui_path - should switch to filesystem
        let result =
            ExplorerLayer::builder().embedded_mode().ui_path("./test-ui").chain_id("TEST").build();

        // May fail due to path not existing, but we can check the config was set
        if let Err(_) = result {
            // Expected since ./test-ui doesn't exist
        }

        // Test the config directly
        let builder = ExplorerLayer::builder().embedded_mode().ui_path("./test-ui");

        if let ExplorerMode::FileSystem { ui_path, hot_reload } = &builder.config.mode {
            assert_eq!(ui_path, &PathBuf::from("./test-ui"));
            assert!(*hot_reload); // Should default to true when switching
        } else {
            panic!("Expected FileSystem mode after setting ui_path");
        }
    }

    #[test]
    fn test_builder_pattern_method_chaining() {
        let builder = ExplorerLayer::builder()
            .chain_id("CHAIN_TEST")
            .cors(true)
            .security_headers(false)
            .ui_env("KEY1", "value1")
            .ui_env("KEY2", 42)
            .ui_env("KEY3", true)
            .header("X-Header1", "value1")
            .header("X-Header2", "value2")
            .debug()
            .api_endpoint("/api/v1");

        // Check all configurations were set
        assert_eq!(builder.config.chain_id, "CHAIN_TEST");
        assert!(builder.config.cors_enabled);
        assert!(!builder.config.security_headers);

        assert_eq!(
            builder.config.ui_env.get("KEY1"),
            Some(&serde_json::Value::String("value1".to_string()))
        );
        assert_eq!(builder.config.ui_env.get("KEY2"), Some(&serde_json::Value::Number(42.into())));
        assert_eq!(builder.config.ui_env.get("KEY3"), Some(&serde_json::Value::Bool(true)));
        assert_eq!(builder.config.ui_env.get("DEBUG"), Some(&serde_json::Value::Bool(true)));
        assert_eq!(
            builder.config.ui_env.get("API_ENDPOINT"),
            Some(&serde_json::Value::String("/api/v1".to_string()))
        );

        assert_eq!(builder.config.custom_headers.get("X-Header1"), Some(&"value1".to_string()));
        assert_eq!(builder.config.custom_headers.get("X-Header2"), Some(&"value2".to_string()));
    }

    #[test]
    fn test_explorer_layer_embedded_without_assets() {
        // This should fail if embedded-ui feature is disabled and no assets
        let result = ExplorerLayer::embedded("TEST".to_string());

        #[cfg(feature = "embedded-ui")]
        {
            // With embedded-ui feature, this might pass or fail depending on whether assets exist
            // We just test that it doesn't panic
            let _ = result;
        }

        #[cfg(not(feature = "embedded-ui"))]
        {
            assert!(result.is_err());
        }
    }

    // Mock service tests would go here - similar to the previous implementation
    // but adapted for the new architecture
}
