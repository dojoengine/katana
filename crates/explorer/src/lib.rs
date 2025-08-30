//! # Katana Explorer
//!
//! A flexible middleware for serving the [Explorer web app] using the [tower] modular networking
//! stack.
//!
//! Explorer is a developer oriented and stateless block explorer that relies entirely on Katana's
//! JSON-RPC interface.
//!
//! ## Serving Modes
//!
//! - [`ExplorerMode::Embedded`]: Serves pre-built UI assets embedded in the binary (suitable for
//!   production)
//! - [`ExplorerMode::Proxy`]: Proxies requests to an external development server (soon)
//!
//! ## Integration with Tower
//!
//! ```rust,no_run
//! use katana_explorer::ExplorerLayer;
//! use tower::ServiceBuilder;
//!
//! let layer = ExplorerLayer::builder().build()?;
//! let service = ServiceBuilder::new().layer(layer).service_fn(your_main_service);
//! ```
//!
//! [Explorer web app]: https://github.com/cartridge-gg/explorer
//! [tower]: https://docs.rs/tower/0.5.2/tower/

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use http::header::HeaderValue;
use http::{HeaderMap, Request, Response, StatusCode};
#[cfg(feature = "jsonrpsee")]
use jsonrpsee::core::{http_helpers::Body, BoxError};
use serde::{Deserialize, Serialize};
use tower::{Layer, Service};
use tracing::debug;
use url::Url;

/// The default path prefix for the Explorer UI when served by Katana.
const DEFAULT_PATH_PREFIX: &str = "/explorer";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Missing or invalid UI assets when in embedded mode.
    #[error("ui asset not found")]
    AssetNotFound,
}

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

/// Explorer serving mode configuration.
///
/// Determines how the Explorer UI assets are served to clients. Each mode has different
/// performance characteristics and is suited for different deployment scenarios.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServingMode {
    /// Serve pre-built UI assets embedded in the binary.
    ///
    /// **Best for**: Production deployments where assets don't change.
    ///
    /// **Requires**: The `embedded-ui` feature must be enabled and UI assets must be
    /// built during compilation via the build script.
    ///
    /// **Performance**: Fastest serving as assets are loaded from memory.
    Embedded,

    /// Proxy requests to an external development server.
    ///
    /// **Best for**: Development with separate UI development server (e.g., Vite dev server).
    ///
    /// **Status**: Planned feature - currently falls back to embedded mode.
    ///
    /// **Future**: Will support proxying to upstream servers like `http://localhost:3000`.
    Proxy {
        /// URL of the upstream development server.
        upstream_url: Url,
        /// Whether to inject environment variables into HTML responses.
        inject_env: bool,
    },
}

/// Comprehensive configuration for the Explorer UI server.
///
/// This struct contains all settings needed to configure how the Explorer UI is served,
/// including the serving mode, security settings, and custom environment variables.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExplorerConfig {
    /// The serving mode determining how UI assets are provided to clients.
    mode: ServingMode,

    /// Blockchain chain ID to inject into the UI environment.
    ///
    /// This value is made available to the UI via `window.CHAIN_ID` and
    /// `window.KATANA_CONFIG.CHAIN_ID` for blockchain connections.
    chain_id: String,

    /// URL path prefix for all Explorer routes.
    ///
    /// All Explorer requests must start with this prefix. For example, with the default
    /// prefix, the Explorer UI is available at `/explorer` and static assets at
    /// `/explorer/assets/*`.
    path_prefix: String,

    /// Enable Cross-Origin Resource Sharing (CORS) headers.
    ///
    /// **Security Note**: Only enable in development or when explicitly needed.
    /// Enabling CORS allows requests from any origin.
    cors_enabled: bool,

    /// Enable production security headers.
    ///
    /// When enabled, adds headers like `X-Frame-Options`, `X-Content-Type-Options`,
    /// `Content-Security-Policy`, etc.
    ///
    /// **Recommendation**: Always enable in production, disable in development for easier
    /// debugging.
    security_headers: bool,

    /// Enable asset compression.
    compression: bool,

    /// Custom HTTP headers to add to all responses.
    ///
    /// Useful for adding environment-specific headers like `X-Environment: staging`
    /// or API versioning headers.
    headers: HashMap<String, String>,

    /// Custom environment variables to inject into the UI.
    ///
    /// These variables are made available to the UI via `window.KATANA_CONFIG`.
    /// Common use cases include API endpoints, feature flags, and theme settings.
    ///
    /// **Note**: The `CHAIN_ID` and `ENABLE_CONTROLLER` variables are automatically
    /// injected and don't need to be specified here.
    ui_env: HashMap<String, serde_json::Value>,
}

/// Tower layer for serving the Katana Explorer UI.
///
/// This layer intercepts HTTP requests matching the configured path prefix and serves
/// the Explorer UI assets, while passing through all other requests to the inner service.
///
/// ## Path Handling
///
/// - Requests to `{path_prefix}/*` are handled by the Explorer
/// - All other requests pass through to the inner service
/// - Static assets are served directly, SPA routes serve `index.html`
#[derive(Debug, Clone)]
pub struct ExplorerLayer {
    config: ExplorerConfig,
}

impl ExplorerLayer {
    /// Create a builder for building a new [`ExplorerLayer`].
    ///
    /// This is the only way to create an [`ExplorerLayer`]. See [`ExplorerLayerBuilder`] for all
    /// available configuration methods.
    pub fn builder() -> ExplorerLayerBuilder {
        ExplorerLayerBuilder::new()
    }
}

/// Fluent builder for creating [`ExplorerLayer`] with ergonomic configuration.
#[derive(Debug, Clone)]
pub struct ExplorerLayerBuilder {
    config: ExplorerConfig,
}

impl ExplorerLayerBuilder {
    /// Create a new [`ExplorerLayerBuilder`] with default configuration.
    pub fn new() -> Self {
        Self {
            config: ExplorerConfig {
                mode: ServingMode::Embedded,
                chain_id: "".to_string(),
                path_prefix: DEFAULT_PATH_PREFIX.to_string(),
                cors_enabled: false,
                security_headers: true,
                compression: false,
                headers: HashMap::new(),
                ui_env: HashMap::new(),
            },
        }
    }

    /// Set the URL path prefix for all Explorer routes.
    ///
    /// All Explorer requests must start with this prefix. Static assets and UI routes
    /// will be served under this path. The default prefix is `/explorer`.
    ///
    /// ## Arguments
    ///
    /// - `prefix`: URL path prefix (should start with `/`)
    ///
    /// ## Examples
    ///
    /// ```rust,no_run
    /// use katana_explorer::ExplorerLayer;
    ///
    /// // UI will be available at /ui/* instead of /explorer/*
    /// let layer = ExplorerLayer::builder().path_prefix("/ui").development().build()?;
    /// # Ok::<(), katana_explorer::ExplorerError>(())
    /// ```
    ///
    /// ## Important
    ///
    /// - The prefix should not end with `/` (e.g., use `/ui` not `/ui/`)
    /// - Changing the prefix affects all Explorer routes including assets
    /// - Make sure your frontend routing is configured for the new prefix
    pub fn path_prefix<S: Into<String>>(mut self, prefix: S) -> Self {
        self.config.path_prefix = prefix.into();
        self
    }

    /// Use embedded assets mode.
    ///
    /// In this mode, pre-built UI assets are served from memory. The assets must
    /// be embedded during compilation via the build script.
    ///
    /// This is suitable if you need a self-contained binaries where the UI won't change. This means
    /// the the binary must be rebuilt whenever the UI changes.
    pub fn embedded(mut self) -> Self {
        self.config.mode = ServingMode::Embedded;
        self
    }

    /// Use proxy mode.
    ///
    /// This convenience method configures proxy mode with environment variable
    /// injection enabled, which is the common use case for development.
    ///
    /// ## Arguments
    ///
    /// - `upstream_url`: URL string of the upstream development server
    pub fn proxy<U: Into<Url>>(mut self, upstream_url: U) -> Self {
        let upstream_url = upstream_url.into();
        self.config.mode = ServingMode::Proxy { upstream_url, inject_env: false };
        self
    }

    /// Enable or disable Cross-Origin Resource Sharing (CORS).
    ///
    /// When enabled, adds CORS headers that allow requests from any origin (`*`).
    /// This is useful for development but should be used carefully in production.
    ///
    /// ## Arguments
    ///
    /// - `enabled`: Whether to add CORS headers to responses
    ///
    /// ## Security Implications
    ///
    /// - **Development**: Generally safe to enable for local development
    /// - **Production**: Only enable if you need cross-origin access and understand the risks
    /// - **CORS headers added**: `Access-Control-Allow-Origin: *`
    pub fn cors(mut self, enabled: bool) -> Self {
        self.config.cors_enabled = enabled;
        self
    }

    /// Enable or disable production security headers.
    ///
    /// When enabled, adds various security headers to protect against common web
    /// vulnerabilities. These headers are recommended for production but can
    /// interfere with development debugging.
    ///
    /// ## Headers Added
    ///
    /// - `X-Frame-Options: DENY` - Prevents clickjacking
    /// - `X-Content-Type-Options: nosniff` - Prevents MIME sniffing
    /// - `Referrer-Policy: strict-origin-when-cross-origin` - Controls referrer info
    /// - `Content-Security-Policy: ...` - Restricts resource loading
    ///
    /// ## Arguments
    ///
    /// - `enabled`: Whether to add security headers to responses
    pub fn security_headers(mut self, enabled: bool) -> Self {
        self.config.security_headers = enabled;
        self
    }

    /// Add a custom HTTP header to all responses.
    ///
    /// ## Arguments
    ///
    /// - `key`: Header name
    /// - `value`: Header value
    ///
    /// ## Examples
    ///
    /// ```rust,no_run
    /// use katana_explorer::ExplorerLayer;
    ///
    /// let layer = ExplorerLayer::builder()
    ///     .header("X-Environment", "production")
    ///     .header("X-API-Version", "v2.0")
    ///     .header("X-Build-Time", "2024-01-15")
    ///     .build()?;
    /// ```
    pub fn header<K, V>(mut self, key: K, value: V) -> Self
    where
        K: Into<String>,
        V: Into<String>,
    {
        self.config.headers.insert(key.into(), value.into());
        self
    }

    /// Add multiple custom HTTP headers to all responses.
    ///
    /// This is a convenience method for adding multiple headers at once rather
    /// than chaining multiple `.header()` calls.
    ///
    /// ## Arguments
    ///
    /// - `headers`: An iterator of (key, value) pairs where both implement `Into<String>`
    ///
    /// ## Examples
    ///
    /// ```rust,no_run
    /// use std::collections::HashMap;
    ///
    /// use katana_explorer::ExplorerLayer;
    ///
    /// let mut header_map = HashMap::new();
    /// header_map.insert("X-Environment", "staging");
    /// header_map.insert("X-API-Version", "v1.0");
    ///
    /// let layer = ExplorerLayer::builder().headers(header_map).build()?;
    ///
    /// // Or with a Vec of tuples
    /// let headers = vec![("X-Service", "katana-explorer"), ("X-Version", "1.0.0")];
    ///
    /// let layer2 = ExplorerLayer::builder().headers(headers).build()?;
    /// ```
    pub fn headers<I, K, V>(mut self, headers: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        for (key, value) in headers {
            self.config.headers.insert(key.into(), value.into());
        }
        self
    }

    /// Add a UI environment variable.
    ///
    /// UI environment variables are injected into the HTML page and made available
    /// to the frontend JavaScript via `window.KATANA_CONFIG`. This allows passing
    /// configuration and runtime information to the UI.
    ///
    /// ## Examples
    ///
    /// ```rust,no_run
    /// use katana_explorer::ExplorerLayer;
    ///
    /// let layer = ExplorerLayer::builder()
    ///     .env("DEBUG", true)
    ///     .env("API_URL", "http://localhost:8080")
    ///     .env("MAX_RETRIES", 3)
    ///     .env("FEATURES", vec!["feature1", "feature2"])
    ///     .build()?;
    /// ```
    ///
    /// Variables are accessible in the browser as:
    ///
    /// ```javascript
    /// console.log(window.KATANA_CONFIG.DEBUG);      // true
    /// console.log(window.KATANA_CONFIG.API_URL);    // "http://localhost:8080"
    /// console.log(window.KATANA_CONFIG.MAX_RETRIES); // 3
    /// ```
    ///
    /// ## Automatic Variables
    ///
    /// These variables are automatically added and don't need to be set manually:
    /// - `CHAIN_ID`: Set via the `chain_id()` method
    /// - `ENABLE_CONTROLLER`: Always set to `false`
    pub fn env<K, V>(mut self, key: K, value: V) -> Self
    where
        K: Into<String>,
        V: Into<serde_json::Value>,
    {
        self.config.ui_env.insert(key.into(), value.into());
        self
    }

    /// Add multiple UI environment variables.
    ///
    /// This is a convenience method for adding multiple UI environment variables
    /// at once rather than chaining multiple `.ui_env()` calls.
    ///
    /// ## Arguments
    ///
    /// - `envs`: An iterator of (key, value) pairs to add to the UI environment
    ///
    /// ## Examples
    ///
    /// ```rust,no_run
    /// use std::collections::HashMap;
    ///
    /// use katana_explorer::ExplorerLayer;
    ///
    /// let mut env_vars = HashMap::new();
    /// env_vars.insert("DEBUG", true);
    /// env_vars.insert("API_TIMEOUT", 5000);
    /// env_vars.insert("ENVIRONMENT", "staging");
    ///
    /// let layer = ExplorerLayer::builder().ui_envs(env_vars).build()?;
    ///
    /// // Or with a Vec of tuples
    /// let envs = vec![("FEATURE_A", true), ("FEATURE_B", false), ("POLL_INTERVAL", 1000)];
    ///
    /// let layer2 = ExplorerLayer::builder().envs(envs).build()?;
    /// ```
    pub fn envs<I, K, V>(mut self, envs: I) -> Self
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

    /// Enable or disable asset compression.
    ///
    /// When enabled, static assets will be compressed before serving to reduce
    /// bandwidth usage and improve loading times. This feature is planned for
    /// future implementation.
    ///
    /// ## Arguments
    ///
    /// - `enabled`: Whether to enable asset compression
    ///
    /// ## Status
    ///
    /// **Currently not implemented** - this setting is reserved for future use.
    /// Setting this value will not have any effect on current behavior.
    ///
    /// ## Implementation Notes
    ///
    /// When implemented, this will likely support:
    /// - Gzip compression for text assets (HTML, CSS, JS)
    /// - Brotli compression for better compression ratios
    /// - Automatic content-encoding headers
    /// - Pre-compression for embedded assets
    pub fn compression(mut self, enabled: bool) -> Self {
        self.config.compression = enabled;
        self
    }

    /// Build [`ExplorerLayer`] with the current configuration.
    ///
    /// This method validates the configuration and creates the final [`ExplorerLayer`].
    /// Configuration validation includes checking that UI paths exist and assets are
    /// available for the selected mode.
    ///
    /// ## Errors
    ///
    /// - **Embedded mode**: Returns error if `embedded-ui` feature is disabled or no assets exist
    /// - **Proxy mode**: Returns error if the upstream URL is invalid
    pub fn build(self) -> Result<ExplorerLayer, Error> {
        // Validate configuration
        match &self.config.mode {
            ServingMode::Embedded => {
                if EmbeddedAssets::get("index.html").is_none() {
                    return Err(Error::AssetNotFound);
                }
                debug!("Explorer configured to embedded mode");
            }

            ServingMode::Proxy { upstream_url, .. } => {
                debug!(%upstream_url, "Explorer configured to proxy mode");
                unimplemented!("Proxy mode is not yet implemented");
            }
        }

        Ok(ExplorerLayer { config: self.config })
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
}

impl<S> ExplorerService<S> {
    fn new(inner: S, config: ExplorerConfig) -> Self {
        Self { inner, config }
    }

    async fn serve_asset(config: &ExplorerConfig, path: &str) -> Option<Response<Body>> {
        match &config.mode {
            ServingMode::Embedded => Self::serve_embedded(config, path).await,
            ServingMode::Proxy { .. } => unimplemented!("Explorer Proxy mode"),
        }
    }

    async fn serve_embedded(config: &ExplorerConfig, path: &str) -> Option<Response<Body>> {
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
                let injected = Self::inject_environment(config, &html);
                Bytes::from(injected)
            } else {
                Bytes::copy_from_slice(&asset.data)
            };

            debug!("Serving {} from embedded assets", asset_path);
            return Some(Self::create_response(config, content_type, content));
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

        // caching headers
        let cache_control = Self::get_cache_control(content_type);
        response = response.header("Cache-Control", cache_control);

        // security headers
        if config.security_headers {
            for (key, value) in Self::get_security_headers().iter() {
                response = response.header(key, value);
            }
        }

        // CORS headers
        if config.cors_enabled {
            response = response.header("Access-Control-Allow-Origin", "*");
            response = response.header("Access-Control-Allow-Methods", "GET, OPTIONS");
            response = response.header("Access-Control-Allow-Headers", "Content-Type");
        }

        // custom headers (if any)
        for (key, value) in &config.headers {
            response = response.header(key, value);
        }

        response.body(Body::from(content.to_vec())).unwrap()
    }

    fn response_404() -> Response<Body> {
        Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header("Content-Type", "text/plain")
            .body(Body::from("Explorer UI not found"))
            .unwrap()
    }

    fn inject_environment(config: &ExplorerConfig, html: &str) -> String {
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
        let uri_path = req.uri().path();
        if !uri_path.starts_with(&self.config.path_prefix) {
            return Box::pin(self.inner.call(req));
        }

        // Extract the file path after the prefix
        let relative_path = uri_path
            .strip_prefix(&self.config.path_prefix)
            .unwrap_or("")
            .trim_start_matches('/')
            .to_string();

        let config = self.config.clone();

        Box::pin(async move {
            let response = match Self::serve_asset(&config, &relative_path).await {
                Some(response) => response,
                None => Self::response_404(),
            };
            Ok(response)
        })
    }
}

impl<S: Clone> Clone for ExplorerService<S> {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone(), config: self.config.clone() }
    }
}

/// Embedded Explorer UI assets
#[derive(rust_embed::RustEmbed)]
#[folder = "ui/dist"]
struct EmbeddedAssets;

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn get_content_type() {
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
    fn is_static_asset_path() {
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
    fn create_response() {
        // Test with minimal config
        let config = ExplorerConfig {
            mode: ServingMode::Embedded,
            chain_id: "test".to_string(),
            path_prefix: "/explorer".to_string(),
            cors_enabled: false,
            security_headers: false,
            compression: false,
            headers: HashMap::new(),
            ui_env: HashMap::new(),
        };

        let content = Bytes::from("test content");
        let response = ExplorerService::<()>::create_response(
            &config,
            "text/html; charset=utf-8",
            content.clone(),
        );

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get("Content-Type").unwrap(), "text/html; charset=utf-8");
        assert_eq!(response.headers().get("Cache-Control").unwrap(), "no-cache, must-revalidate");

        // with security headers enabled
        let mut config_with_security = config.clone();
        config_with_security.security_headers = true;
        let response = ExplorerService::<()>::create_response(
            &config_with_security,
            "text/html; charset=utf-8",
            content.clone(),
        );

        assert!(response.headers().get("X-Frame-Options").is_some());
        assert!(response.headers().get("X-Content-Type-Options").is_some());
        assert!(response.headers().get("Referrer-Policy").is_some());
        assert!(response.headers().get("Content-Security-Policy").is_some());

        // with CORS enabled
        let mut config_with_cors = config.clone();
        config_with_cors.cors_enabled = true;
        let response = ExplorerService::<()>::create_response(
            &config_with_cors,
            "text/html; charset=utf-8",
            content.clone(),
        );

        assert_eq!(response.headers().get("Access-Control-Allow-Origin").unwrap(), "*");
        assert_eq!(response.headers().get("Access-Control-Allow-Methods").unwrap(), "GET, OPTIONS");
        assert_eq!(response.headers().get("Access-Control-Allow-Headers").unwrap(), "Content-Type");

        //  with custom headers
        let mut config_with_headers = config.clone();
        config_with_headers
            .headers
            .insert("X-Custom-Header".to_string(), "custom-value".to_string());
        config_with_headers
            .headers
            .insert("X-Another-Header".to_string(), "another-value".to_string());
        let response = ExplorerService::<()>::create_response(
            &config_with_headers,
            "text/html; charset=utf-8",
            content.clone(),
        );

        assert_eq!(response.headers().get("X-Custom-Header").unwrap(), "custom-value");
        assert_eq!(response.headers().get("X-Another-Header").unwrap(), "another-value");

        // cache control for different content types
        let response_js = ExplorerService::<()>::create_response(
            &config,
            "application/javascript; charset=utf-8",
            content.clone(),
        );
        assert_eq!(
            response_js.headers().get("Cache-Control").unwrap(),
            "public, max-age=31536000, immutable"
        );

        let response_css = ExplorerService::<()>::create_response(
            &config,
            "text/css; charset=utf-8",
            content.clone(),
        );
        assert_eq!(
            response_css.headers().get("Cache-Control").unwrap(),
            "public, max-age=31536000, immutable"
        );

        let response_img =
            ExplorerService::<()>::create_response(&config, "image/png", content.clone());
        assert_eq!(response_img.headers().get("Cache-Control").unwrap(), "public, max-age=3600");
    }

    #[test]
    fn builder_pattern_method_chaining() {
        let builder = ExplorerLayer::builder()
            .cors(true)
            .security_headers(false)
            .env("KEY1", "value1")
            .env("KEY2", 42)
            .env("KEY3", true)
            .header("X-Header1", "value1")
            .header("X-Header2", "value2");

        assert!(builder.config.cors_enabled);
        assert!(!builder.config.security_headers);

        assert_eq!(
            builder.config.ui_env.get("KEY1"),
            Some(&serde_json::Value::String("value1".to_string()))
        );
        assert_eq!(builder.config.ui_env.get("KEY2"), Some(&serde_json::Value::Number(42.into())));
        assert_eq!(builder.config.ui_env.get("KEY3"), Some(&serde_json::Value::Bool(true)));

        assert_eq!(builder.config.headers.get("X-Header1"), Some(&"value1".to_string()));
        assert_eq!(builder.config.headers.get("X-Header2"), Some(&"value2".to_string()));
    }
}
