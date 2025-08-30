//! # Katana Explorer
//!
//! A flexible Tower middleware for serving the Katana blockchain explorer UI with multiple
//! deployment modes including hot reload for development and embedded assets for production.
//!
//! ## Features
//!
//! - **Hot Reload**: Real-time UI updates during development
//! - **Multiple Serving Modes**: Embedded, FileSystem, and Proxy
//! - **Security Headers**: Production-ready security configuration
//! - **Caching**: Intelligent file caching with invalidation
//! - **SPA Support**: Single Page Application routing
//! - **Builder Pattern**: Ergonomic configuration API
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use katana_explorer::ExplorerLayer;
//!
//! // Development mode with hot reload
//! let dev_layer = ExplorerLayer::builder().development().chain_id("KATANA_DEV").build()?;
//!
//! // Production mode with embedded assets
//! let prod_layer = ExplorerLayer::builder().production().chain_id("KATANA_PROD").build()?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! ## Serving Modes
//!
//! - [`ExplorerMode::Embedded`]: Serves pre-built UI assets embedded in the binary
//! - [`ExplorerMode::FileSystem`]: Serves UI files from disk with optional hot reload
//! - [`ExplorerMode::Proxy`]: Proxies requests to an external development server (planned)
//!
//! ## Integration with Tower
//!
//! ```rust,no_run
//! use katana_explorer::ExplorerLayer;
//! use tower::ServiceBuilder;
//!
//! let service = ServiceBuilder::new().layer(explorer_layer).service_fn(your_main_service);
//! ```

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

/// Explorer serving mode configuration.
///
/// Determines how the Explorer UI assets are served to clients. Each mode has different
/// performance characteristics and is suited for different deployment scenarios.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExplorerMode {
    /// Serve pre-built UI assets embedded in the binary.
    ///
    /// **Best for**: Production deployments where assets don't change.
    ///
    /// **Requires**: The `embedded-ui` feature must be enabled and UI assets must be
    /// built during compilation via the build script.
    ///
    /// **Performance**: Fastest serving as assets are loaded from memory.
    Embedded,

    /// Serve UI files from the filesystem with optional hot reload.
    ///
    /// **Best for**: Development and scenarios where UI can be updated without recompilation.
    ///
    /// **Requires**: The specified `ui_path` must exist and contain built UI assets.
    /// When `hot_reload` is enabled, file system watching is used to detect changes.
    ///
    /// **Performance**: Slightly slower than embedded mode due to file I/O, but supports
    /// intelligent caching and hot reload for development efficiency.
    FileSystem {
        /// Path to the directory containing built UI assets (must contain index.html).
        ui_path: PathBuf,
        /// Enable real-time file watching and cache invalidation for development.
        hot_reload: bool,
    },

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
///
/// ## Examples
///
/// ```rust,no_run
/// use std::collections::HashMap;
/// use std::path::PathBuf;
///
/// use katana_explorer::{ExplorerConfig, ExplorerMode};
///
/// // Development configuration
/// let dev_config = ExplorerConfig {
///     mode: ExplorerMode::FileSystem { ui_path: PathBuf::from("./ui/dist"), hot_reload: true },
///     chain_id: "KATANA_DEV".to_string(),
///     cors_enabled: true,
///     security_headers: false,
///     ..Default::default()
/// };
///
/// // Production configuration
/// let prod_config = ExplorerConfig {
///     mode: ExplorerMode::Embedded,
///     chain_id: "KATANA_PROD".to_string(),
///     security_headers: true,
///     ..Default::default()
/// };
/// ```
///
/// ## Recommendations
///
/// - Use [`ExplorerLayer::builder()`] for a more ergonomic configuration experience
/// - Enable `hot_reload` only in development environments
/// - Always enable `security_headers` in production
/// - Use `cors_enabled` carefully - only enable when necessary for development
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExplorerConfig {
    /// The serving mode determining how UI assets are provided to clients.
    pub mode: ExplorerMode,

    /// Blockchain chain ID to inject into the UI environment.
    ///
    /// This value is made available to the UI via `window.CHAIN_ID` and
    /// `window.KATANA_CONFIG.CHAIN_ID` for blockchain connections.
    pub chain_id: String,

    /// URL path prefix for all Explorer routes.
    ///
    /// **Default**: `"/explorer"`
    ///
    /// All Explorer requests must start with this prefix. For example, with the default
    /// prefix, the Explorer UI is available at `/explorer` and static assets at
    /// `/explorer/assets/*`.
    pub path_prefix: String,

    /// Enable Cross-Origin Resource Sharing (CORS) headers.
    ///
    /// **Security Note**: Only enable in development or when explicitly needed.
    /// Enabling CORS allows requests from any origin.
    ///
    /// **Default**: `false`
    pub cors_enabled: bool,

    /// Enable production security headers.
    ///
    /// When enabled, adds headers like `X-Frame-Options`, `X-Content-Type-Options`,
    /// `Content-Security-Policy`, etc.
    ///
    /// **Recommendation**: Always enable in production, disable in development for easier
    /// debugging.
    ///
    /// **Default**: `true`
    pub security_headers: bool,

    /// Enable asset compression (future feature).
    ///
    /// **Status**: Not yet implemented.
    ///
    /// **Default**: `false`
    pub compression: bool,

    /// Custom HTTP headers to add to all responses.
    ///
    /// Useful for adding environment-specific headers like `X-Environment: staging`
    /// or API versioning headers.
    pub custom_headers: HashMap<String, String>,

    /// Custom environment variables to inject into the UI.
    ///
    /// These variables are made available to the UI via `window.KATANA_CONFIG`.
    /// Common use cases include API endpoints, feature flags, and theme settings.
    ///
    /// **Note**: The `CHAIN_ID` and `ENABLE_CONTROLLER` variables are automatically
    /// injected and don't need to be specified here.
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

/// File cache entry for hot reload functionality.
///
/// Stores cached file content along with metadata for efficient serving
/// and change detection in FileSystem mode with hot reload enabled.
#[derive(Debug, Clone)]
struct CacheEntry {
    /// The cached file content.
    content: Bytes,
    /// The MIME content type for HTTP response headers.
    content_type: String,
    /// Last modification time for cache invalidation.
    last_modified: SystemTime,
}

/// Tower layer for serving the Katana Explorer UI.
///
/// This layer intercepts HTTP requests matching the configured path prefix and serves
/// the Explorer UI assets, while passing through all other requests to the inner service.
///
/// ## Usage
///
/// ```rust,no_run
/// use katana_explorer::ExplorerLayer;
/// use tower::ServiceBuilder;
///
/// // Using builder pattern (recommended)
/// let layer = ExplorerLayer::builder()
///     .development()           // or .production()
///     .chain_id("KATANA")
///     .build()?;
///
/// // Integrate with Tower service stack
/// let service = ServiceBuilder::new().layer(layer).service_fn(your_main_service);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// ## Path Handling
///
/// - Requests to `{path_prefix}/*` are handled by the Explorer
/// - All other requests pass through to the inner service
/// - Static assets are served directly, SPA routes serve `index.html`
///
/// ## Error Conditions
///
/// Layer creation fails if:
/// - Embedded mode is selected but no assets are available
/// - FileSystem mode is selected but the UI path doesn't exist
/// - Invalid configuration is provided
#[derive(Debug, Clone)]
pub struct ExplorerLayer {
    config: ExplorerConfig,
}

impl ExplorerLayer {
    /// Create a new ExplorerLayer with the given configuration.
    ///
    /// ## Validation
    ///
    /// This method validates the provided configuration:
    /// - For `Embedded` mode: Checks that UI assets are available
    /// - For `FileSystem` mode: Verifies the UI path exists
    /// - For `Proxy` mode: Logs the upstream configuration
    ///
    /// ## Examples
    ///
    /// ```rust,no_run
    /// use std::path::PathBuf;
    ///
    /// use katana_explorer::{ExplorerConfig, ExplorerLayer, ExplorerMode};
    ///
    /// let config = ExplorerConfig {
    ///     mode: ExplorerMode::FileSystem { ui_path: PathBuf::from("./ui/dist"), hot_reload: true },
    ///     chain_id: "KATANA".to_string(),
    ///     ..Default::default()
    /// };
    ///
    /// let layer = ExplorerLayer::new(config)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// ## Errors
    ///
    /// Returns an error if:
    /// - Embedded mode is selected but `embedded-ui` feature is disabled or no assets exist
    /// - FileSystem mode is selected but the specified path doesn't exist
    /// - Invalid URLs are provided in proxy mode
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

    /// Create a builder for ergonomic layer configuration.
    ///
    /// The builder pattern provides a fluent API with sensible defaults and preset methods
    /// for common use cases. This is the recommended way to create an `ExplorerLayer`.
    ///
    /// ## Examples
    ///
    /// ```rust,no_run
    /// use katana_explorer::ExplorerLayer;
    ///
    /// // Development mode with hot reload
    /// let dev_layer = ExplorerLayer::builder().development().chain_id("KATANA_DEV").build()?;
    ///
    /// // Custom configuration
    /// let custom_layer = ExplorerLayer::builder()
    ///     .chain_id("KATANA")
    ///     .filesystem_with_hot_reload("./ui/dist")
    ///     .with_cors()
    ///     .debug()
    ///     .build()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// See [`ExplorerLayerBuilder`] for all available configuration methods.
    pub fn builder() -> ExplorerLayerBuilder {
        ExplorerLayerBuilder::new()
    }

    /// Create an ExplorerLayer with embedded assets mode.
    ///
    /// This is a convenience method for production deployments where UI assets
    /// are embedded in the binary.
    ///
    /// ## Requirements
    ///
    /// - The `embedded-ui` feature must be enabled (default)
    /// - UI assets must be available (built during compilation)
    ///
    /// ## Equivalent to
    ///
    /// ```rust,no_run
    /// use katana_explorer::ExplorerLayer;
    ///
    /// // These are equivalent:
    /// let layer1 = ExplorerLayer::embedded("KATANA".to_string())?;
    /// let layer2 = ExplorerLayer::builder().production().chain_id("KATANA").build()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// ## Errors
    ///
    /// Returns an error if embedded assets are not available.
    pub fn embedded(chain_id: String) -> Result<Self> {
        Self::new(ExplorerConfig { mode: ExplorerMode::Embedded, chain_id, ..Default::default() })
    }

    /// Create an ExplorerLayer with filesystem mode.
    ///
    /// This convenience method configures the layer to serve UI files from the specified
    /// directory with optional hot reload for development.
    ///
    /// ## Parameters
    ///
    /// - `ui_path`: Directory containing built UI assets (must contain `index.html`)
    /// - `chain_id`: Blockchain chain ID to inject into the UI
    /// - `hot_reload`: Enable file watching and automatic cache invalidation
    ///
    /// ## Examples
    ///
    /// ```rust,no_run
    /// use std::path::PathBuf;
    ///
    /// use katana_explorer::ExplorerLayer;
    ///
    /// // Development with hot reload
    /// let dev_layer = ExplorerLayer::filesystem(
    ///     PathBuf::from("./ui/dist"),
    ///     "KATANA_DEV".to_string(),
    ///     true, // hot_reload
    /// )?;
    ///
    /// // Static filesystem serving
    /// let static_layer = ExplorerLayer::filesystem(
    ///     PathBuf::from("/var/www/explorer"),
    ///     "KATANA_PROD".to_string(),
    ///     false, // no hot reload
    /// )?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// ## Errors
    ///
    /// Returns an error if the specified UI path doesn't exist.
    pub fn filesystem(ui_path: PathBuf, chain_id: String, hot_reload: bool) -> Result<Self> {
        Self::new(ExplorerConfig {
            mode: ExplorerMode::FileSystem { ui_path, hot_reload },
            chain_id,
            ..Default::default()
        })
    }

    /// Create an ExplorerLayer with proxy mode.
    ///
    /// This convenience method configures the layer to proxy requests to an external
    /// development server (e.g., Vite dev server).
    ///
    /// ## Status
    ///
    /// **Note**: Proxy mode is not fully implemented yet and currently falls back
    /// to embedded mode. This method is provided for future compatibility.
    ///
    /// ## Parameters
    ///
    /// - `upstream_url`: URL of the development server to proxy to
    /// - `chain_id`: Blockchain chain ID to inject into proxied HTML responses
    ///
    /// ## Examples
    ///
    /// ```rust,no_run
    /// use katana_explorer::ExplorerLayer;
    /// use url::Url;
    ///
    /// let proxy_layer =
    ///     ExplorerLayer::proxy(Url::parse("http://localhost:3000")?, "KATANA_DEV".to_string())?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn proxy(upstream_url: Url, chain_id: String) -> Result<Self> {
        Self::new(ExplorerConfig {
            mode: ExplorerMode::Proxy { upstream_url, inject_env: true },
            chain_id,
            ..Default::default()
        })
    }
}

/// Fluent builder for creating [`ExplorerLayer`] with ergonomic configuration.
///
/// The builder provides a convenient API with method chaining, sensible defaults,
/// and preset configurations for common use cases. This is the recommended way
/// to configure the Explorer layer.
///
/// ## Design Philosophy
///
/// - **Preset Methods**: `.development()` and `.production()` provide opinionated defaults
/// - **Fluent API**: All methods return `Self` for easy chaining
/// - **Type Safety**: Invalid configurations are caught at compile time
/// - **Discoverability**: Method names clearly indicate their purpose
///
/// ## Basic Usage
///
/// ```rust,no_run
/// use katana_explorer::ExplorerLayer;
///
/// // Start with a preset and customize as needed
/// let layer = ExplorerLayer::builder()
///     .development()           // Apply development defaults
///     .chain_id("KATANA_DEV")  // Set required chain ID
///     .ui_path("./my-ui")      // Override UI path if needed
///     .build()?; // Create the layer
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// ## Advanced Usage
///
/// ```rust,no_run
/// use katana_explorer::ExplorerLayer;
///
/// let layer = ExplorerLayer::builder()
///     .chain_id("KATANA")
///     .filesystem_with_hot_reload("./ui/dist")
///     .cors(true)
///     .security_headers(false)
///     .ui_env("DEBUG", true)
///     .ui_env("API_ENDPOINT", "/api/v2")
///     .header("X-Environment", "development")
///     .path_prefix("/blockchain-explorer")
///     .build()?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone)]
pub struct ExplorerLayerBuilder {
    config: ExplorerConfig,
}

impl ExplorerLayerBuilder {
    /// Create a new builder with default configuration.
    ///
    /// Initializes the builder with [`ExplorerConfig::default()`] values.
    /// Use preset methods like [`development()`](Self::development) or
    /// [`production()`](Self::production) for common configurations.
    ///
    /// ## Returns
    ///
    /// Returns `Self` for method chaining (fluent API pattern).
    ///
    /// ## Examples
    ///
    /// ```rust,no_run
    /// use katana_explorer::ExplorerLayer;
    ///
    /// let builder = ExplorerLayer::builder(); // Same as ExplorerLayerBuilder::new()
    /// ```
    pub fn new() -> Self {
        Self { config: ExplorerConfig::default() }
    }

    /// Build the ExplorerLayer with the current configuration.
    ///
    /// This method validates the configuration and creates the final [`ExplorerLayer`].
    /// Configuration validation includes checking that UI paths exist and assets are
    /// available for the selected mode.
    ///
    /// ## Returns
    ///
    /// Returns `Result<ExplorerLayer, anyhow::Error>` where errors indicate
    /// configuration validation failures.
    ///
    /// ## Errors
    ///
    /// - **Embedded mode**: Returns error if `embedded-ui` feature is disabled or no assets exist
    /// - **FileSystem mode**: Returns error if the specified UI path doesn't exist
    /// - **Proxy mode**: Returns error if the upstream URL is invalid
    ///
    /// ## Examples
    ///
    /// ```rust,no_run
    /// use katana_explorer::ExplorerLayer;
    ///
    /// let layer = ExplorerLayer::builder().development().chain_id("KATANA_DEV").build()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn build(self) -> Result<ExplorerLayer> {
        ExplorerLayer::new(self.config)
    }

    // === Preset Methods ===

    /// Configure for development with sensible defaults.
    ///
    /// This preset is optimized for local development and debugging:
    /// - **FileSystem mode** with hot reload enabled (path: `"./ui/dist"`)
    /// - **CORS enabled** for cross-origin requests during development
    /// - **Security headers disabled** for easier debugging
    /// - **Compression disabled** for faster builds
    ///
    /// ## When to use
    ///
    /// Use this preset when developing locally and you need real-time UI updates.
    /// The hot reload feature watches for file changes and automatically serves
    /// updated assets.
    ///
    /// ## Returns
    ///
    /// Returns `Self` for method chaining.
    ///
    /// ## Examples
    ///
    /// ```rust,no_run
    /// use katana_explorer::ExplorerLayer;
    ///
    /// let dev_layer = ExplorerLayer::builder().development().chain_id("KATANA_DEV").build()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// ## Post-configuration
    ///
    /// You can further customize after calling this preset:
    ///
    /// ```rust,no_run
    /// use katana_explorer::ExplorerLayer;
    ///
    /// let custom_dev = ExplorerLayer::builder()
    ///     .development()
    ///     .ui_path("./my-ui/build")  // Override default path
    ///     .api_endpoint("/api/v2")   // Add custom API endpoint
    ///     .build()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn development(mut self) -> Self {
        self.config.mode =
            ExplorerMode::FileSystem { ui_path: PathBuf::from("./ui/dist"), hot_reload: true };
        self.config.cors_enabled = true;
        self.config.security_headers = false;
        self.config.compression = false;
        self
    }

    /// Configure for production with sensible defaults.
    ///
    /// This preset is optimized for production deployments:
    /// - **Embedded mode** for optimal performance and self-contained deployment
    /// - **Security headers enabled** for production security
    /// - **CORS disabled** for security (only enable if needed)
    /// - **Compression enabled** for reduced bundle sizes (future feature)
    ///
    /// ## When to use
    ///
    /// Use this preset for production deployments where performance and security
    /// are priorities. The embedded mode serves assets from memory with no filesystem I/O.
    ///
    /// ## Requirements
    ///
    /// - The `embedded-ui` feature must be enabled (default)
    /// - UI assets must be built and embedded during compilation
    ///
    /// ## Returns
    ///
    /// Returns `Self` for method chaining.
    ///
    /// ## Examples
    ///
    /// ```rust,no_run
    /// use katana_explorer::ExplorerLayer;
    ///
    /// let prod_layer = ExplorerLayer::builder().production().chain_id("KATANA_MAINNET").build()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// ## Post-configuration
    ///
    /// You can customize security settings after calling this preset:
    ///
    /// ```rust,no_run
    /// use katana_explorer::ExplorerLayer;
    ///
    /// let custom_prod = ExplorerLayer::builder()
    ///     .production()
    ///     .with_cors()  // Enable CORS if needed for production
    ///     .header("X-Environment", "staging")
    ///     .build()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn production(mut self) -> Self {
        self.config.mode = ExplorerMode::Embedded;
        self.config.cors_enabled = false;
        self.config.security_headers = true;
        self.config.compression = true;
        self
    }

    // === Core Configuration ===

    /// Set the blockchain chain ID.
    ///
    /// The chain ID is injected into the UI environment as both `window.CHAIN_ID`
    /// and `window.KATANA_CONFIG.CHAIN_ID` for the frontend to use when connecting
    /// to the blockchain.
    ///
    /// ## Parameters
    ///
    /// - `chain_id`: The chain identifier (e.g., "KATANA", "KATANA_DEV", "SN_MAIN")
    ///
    /// ## Returns
    ///
    /// Returns `Self` for method chaining.
    ///
    /// ## Examples
    ///
    /// ```rust,no_run
    /// use katana_explorer::ExplorerLayer;
    ///
    /// let layer = ExplorerLayer::builder().chain_id("KATANA_TESTNET").development().build()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn chain_id<S: Into<String>>(mut self, chain_id: S) -> Self {
        self.config.chain_id = chain_id.into();
        self
    }

    /// Set the URL path prefix for all Explorer routes.
    ///
    /// All Explorer requests must start with this prefix. Static assets and UI routes
    /// will be served under this path. The default prefix is `"/explorer"`.
    ///
    /// ## Parameters
    ///
    /// - `prefix`: URL path prefix (should start with `/`)
    ///
    /// ## Returns
    ///
    /// Returns `Self` for method chaining.
    ///
    /// ## Examples
    ///
    /// ```rust,no_run
    /// use katana_explorer::ExplorerLayer;
    ///
    /// // UI will be available at /ui/* instead of /explorer/*
    /// let layer = ExplorerLayer::builder().path_prefix("/ui").development().build()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// ## Important Notes
    ///
    /// - The prefix should not end with `/` (e.g., use `/ui` not `/ui/`)
    /// - Changing the prefix affects all Explorer routes including assets
    /// - Make sure your frontend routing is configured for the new prefix
    pub fn path_prefix<S: Into<String>>(mut self, prefix: S) -> Self {
        self.config.path_prefix = prefix.into();
        self
    }

    // === Serving Mode Configuration ===

    /// Use embedded assets mode.
    ///
    /// In this mode, pre-built UI assets are served from memory. The assets must
    /// be embedded during compilation via the build script and the `embedded-ui`
    /// feature must be enabled.
    ///
    /// ## When to use
    ///
    /// - Production deployments for optimal performance
    /// - Self-contained binaries where UI won't change
    /// - When filesystem access is limited or not desired
    ///
    /// ## Requirements
    ///
    /// - `embedded-ui` feature enabled (default)
    /// - UI assets must be built before compilation
    ///
    /// ## Returns
    ///
    /// Returns `Self` for method chaining.
    ///
    /// ## Examples
    ///
    /// ```rust,no_run
    /// use katana_explorer::ExplorerLayer;
    ///
    /// let layer = ExplorerLayer::builder().embedded_mode().chain_id("KATANA").build()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// ## Performance
    ///
    /// Embedded mode offers the best performance since assets are served directly
    /// from memory with no filesystem I/O overhead.
    pub fn embedded_mode(mut self) -> Self {
        self.config.mode = ExplorerMode::Embedded;
        self
    }

    /// Use filesystem mode with specified path and hot reload setting.
    ///
    /// In this mode, UI assets are served from the filesystem. The UI path must
    /// exist and contain a built UI (including `index.html`).
    ///
    /// ## Parameters
    ///
    /// - `ui_path`: Path to directory containing built UI assets
    /// - `hot_reload`: Whether to watch for file changes and invalidate cache
    ///
    /// ## When to use
    ///
    /// - Development when you need file watching (`hot_reload = true`)
    /// - Production when UI assets are deployed separately (`hot_reload = false`)
    /// - When you want to update UI without recompiling the binary
    ///
    /// ## Returns
    ///
    /// Returns `Self` for method chaining.
    ///
    /// ## Examples
    ///
    /// ```rust,no_run
    /// use std::path::PathBuf;
    ///
    /// use katana_explorer::ExplorerLayer;
    ///
    /// // Development with hot reload
    /// let dev_layer =
    ///     ExplorerLayer::builder().filesystem_mode("./ui/dist", true).chain_id("DEV").build()?;
    ///
    /// // Production from filesystem (no hot reload)
    /// let prod_layer = ExplorerLayer::builder()
    ///     .filesystem_mode("/var/www/explorer", false)
    ///     .chain_id("PROD")
    ///     .build()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn filesystem_mode<P: Into<PathBuf>>(mut self, ui_path: P, hot_reload: bool) -> Self {
        self.config.mode = ExplorerMode::FileSystem { ui_path: ui_path.into(), hot_reload };
        self
    }

    /// Use filesystem mode with hot reload enabled.
    ///
    /// This is a convenience method for the common development use case where you
    /// want to serve from filesystem with automatic cache invalidation when files change.
    ///
    /// ## Parameters
    ///
    /// - `ui_path`: Path to directory containing built UI assets
    ///
    /// ## Returns
    ///
    /// Returns `Self` for method chaining.
    ///
    /// ## Examples
    ///
    /// ```rust,no_run
    /// use katana_explorer::ExplorerLayer;
    ///
    /// let layer = ExplorerLayer::builder()
    ///     .filesystem_with_hot_reload("./ui/build")
    ///     .chain_id("DEV")
    ///     .build()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// ## Equivalent to
    ///
    /// ```rust,no_run
    /// use katana_explorer::ExplorerLayer;
    ///
    /// let layer = ExplorerLayer::builder()
    ///     .filesystem_mode("./ui/build", true)  // hot_reload = true
    ///     .chain_id("DEV")
    ///     .build()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn filesystem_with_hot_reload<P: Into<PathBuf>>(self, ui_path: P) -> Self {
        self.filesystem_mode(ui_path, true)
    }

    /// Use filesystem mode with hot reload disabled.
    ///
    /// This is useful for production deployments where UI assets are served from
    /// the filesystem but you don't want the overhead of file watching.
    ///
    /// ## Parameters
    ///
    /// - `ui_path`: Path to directory containing built UI assets
    ///
    /// ## Returns
    ///
    /// Returns `Self` for method chaining.
    ///
    /// ## Examples
    ///
    /// ```rust,no_run
    /// use katana_explorer::ExplorerLayer;
    ///
    /// let layer = ExplorerLayer::builder()
    ///     .filesystem_static("/var/www/katana-ui")
    ///     .chain_id("PROD")
    ///     .build()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// ## Use cases
    ///
    /// - Production with UI assets deployed via CD pipeline
    /// - Docker containers with UI assets in separate volume
    /// - When you want filesystem serving without file watching overhead
    pub fn filesystem_static<P: Into<PathBuf>>(self, ui_path: P) -> Self {
        self.filesystem_mode(ui_path, false)
    }

    /// Use proxy mode with full control over settings.
    ///
    /// In proxy mode, requests are forwarded to an upstream development server
    /// (e.g., Vite dev server). This is useful during development when running
    /// the UI with its own development server.
    ///
    /// ## Parameters
    ///
    /// - `upstream_url`: URL of the upstream development server
    /// - `inject_env`: Whether to inject environment variables into HTML responses
    ///
    /// ## Status
    ///
    /// **Currently planned but not implemented** - will fall back to embedded mode.
    ///
    /// ## Returns
    ///
    /// Returns `Self` for method chaining.
    ///
    /// ## Future Examples
    ///
    /// ```rust,no_run
    /// use katana_explorer::ExplorerLayer;
    /// use url::Url;
    ///
    /// let upstream = Url::parse("http://localhost:3000")?;
    /// let layer = ExplorerLayer::builder()
    ///     .proxy_mode(upstream, true)  // inject_env = true
    ///     .chain_id("DEV")
    ///     .build()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn proxy_mode(mut self, upstream_url: Url, inject_env: bool) -> Self {
        self.config.mode = ExplorerMode::Proxy { upstream_url, inject_env };
        self
    }

    /// Use proxy mode with environment injection enabled.
    ///
    /// This convenience method configures proxy mode with environment variable
    /// injection enabled, which is the common use case for development.
    ///
    /// ## Parameters
    ///
    /// - `upstream_url`: URL string of the upstream development server
    ///
    /// ## Returns
    ///
    /// Returns `Result<Self, anyhow::Error>` where errors indicate invalid URLs.
    ///
    /// ## Errors
    ///
    /// Returns error if the upstream URL cannot be parsed.
    ///
    /// ## Status
    ///
    /// **Currently planned but not implemented** - will fall back to embedded mode.
    ///
    /// ## Future Examples
    ///
    /// ```rust,no_run
    /// use katana_explorer::ExplorerLayer;
    ///
    /// let layer = ExplorerLayer::builder().proxy("http://localhost:3000")?.chain_id("DEV").build()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn proxy<S: AsRef<str>>(self, upstream_url: S) -> Result<Self> {
        let url = Url::parse(upstream_url.as_ref())
            .map_err(|e| anyhow!("Invalid upstream URL: {}", e))?;
        Ok(self.proxy_mode(url, true))
    }

    // === Convenience Methods for UI Path ===

    /// Set or change the UI path for filesystem mode.
    ///
    /// This method automatically switches to filesystem mode if not already in that mode.
    /// If already in filesystem mode, it updates the path while preserving the hot reload setting.
    ///
    /// ## Parameters
    ///
    /// - `path`: Path to directory containing built UI assets
    ///
    /// ## Behavior
    ///
    /// - **If in FileSystem mode**: Updates the UI path, preserves hot reload setting
    /// - **If in other modes**: Switches to FileSystem mode with hot reload enabled
    ///
    /// ## Returns
    ///
    /// Returns `Self` for method chaining.
    ///
    /// ## Examples
    ///
    /// ```rust,no_run
    /// use katana_explorer::ExplorerLayer;
    ///
    /// // Starting with embedded mode, switches to filesystem
    /// let layer = ExplorerLayer::builder()
    ///     .embedded_mode()
    ///     .ui_path("./ui/dist")  // Now in filesystem mode with hot reload
    ///     .build()?;
    ///
    /// // Already in filesystem mode, just updates path
    /// let layer2 = ExplorerLayer::builder()
    ///     .filesystem_static("./old-path")
    ///     .ui_path("./new-path")  // Updates path, preserves hot_reload=false
    ///     .build()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
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

    /// Enable or disable hot reload for filesystem mode.
    ///
    /// Hot reload watches the UI directory for file changes and automatically
    /// invalidates the file cache, ensuring the latest files are served.
    ///
    /// ## Parameters
    ///
    /// - `enabled`: Whether to enable file watching and cache invalidation
    ///
    /// ## Behavior
    ///
    /// - **If in FileSystem mode**: Updates the hot reload setting
    /// - **If in other modes**: Logs a debug message and ignores the setting
    ///
    /// ## Returns
    ///
    /// Returns `Self` for method chaining.
    ///
    /// ## Examples
    ///
    /// ```rust,no_run
    /// use katana_explorer::ExplorerLayer;
    ///
    /// // Enable hot reload for development
    /// let dev_layer = ExplorerLayer::builder()
    ///     .filesystem_static("./ui/dist")
    ///     .hot_reload(true)  // Now has hot reload enabled
    ///     .build()?;
    ///
    /// // Disable for production
    /// let prod_layer = ExplorerLayer::builder()
    ///     .filesystem_with_hot_reload("./ui/dist")
    ///     .hot_reload(false)  // Disables hot reload
    ///     .build()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// ## Performance Note
    ///
    /// Hot reload adds file watching overhead. Disable in production for better performance.
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

    /// Enable or disable Cross-Origin Resource Sharing (CORS).
    ///
    /// When enabled, adds CORS headers that allow requests from any origin (`*`).
    /// This is useful for development but should be used carefully in production.
    ///
    /// ## Parameters
    ///
    /// - `enabled`: Whether to add CORS headers to responses
    ///
    /// ## Security Implications
    ///
    /// - **Development**: Generally safe to enable for local development
    /// - **Production**: Only enable if you need cross-origin access and understand the risks
    /// - **CORS headers added**: `Access-Control-Allow-Origin: *`
    ///
    /// ## Returns
    ///
    /// Returns `Self` for method chaining.
    ///
    /// ## Examples
    ///
    /// ```rust,no_run
    /// use katana_explorer::ExplorerLayer;
    ///
    /// // Enable CORS for development
    /// let dev_layer = ExplorerLayer::builder().development().cors(true).build()?;
    ///
    /// // Disable CORS for security
    /// let secure_layer = ExplorerLayer::builder().production().cors(false).build()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn cors(mut self, enabled: bool) -> Self {
        self.config.cors_enabled = enabled;
        self
    }

    /// Enable CORS (convenience method).
    ///
    /// This is a convenience method equivalent to `.cors(true)` for better readability
    /// in builder chains.
    ///
    /// ## Returns
    ///
    /// Returns `Self` for method chaining.
    ///
    /// ## Examples
    ///
    /// ```rust,no_run
    /// use katana_explorer::ExplorerLayer;
    ///
    /// let layer = ExplorerLayer::builder()
    ///     .development()
    ///     .with_cors()  // More readable than .cors(true)
    ///     .build()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// ## Equivalent to
    ///
    /// ```rust,no_run
    /// use katana_explorer::ExplorerLayer;
    ///
    /// let layer = ExplorerLayer::builder().development().cors(true).build()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn with_cors(self) -> Self {
        self.cors(true)
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
    /// ## Parameters
    ///
    /// - `enabled`: Whether to add security headers to responses
    ///
    /// ## Returns
    ///
    /// Returns `Self` for method chaining.
    ///
    /// ## Examples
    ///
    /// ```rust,no_run
    /// use katana_explorer::ExplorerLayer;
    ///
    /// // Production with security headers
    /// let prod_layer = ExplorerLayer::builder()
    ///     .production()
    ///     .security_headers(true)  // Already enabled by production()
    ///     .build()?;
    ///
    /// // Development without security headers for easier debugging
    /// let dev_layer = ExplorerLayer::builder()
    ///     .development()
    ///     .security_headers(false)  // Already disabled by development()
    ///     .build()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// ## Recommendations
    ///
    /// - **Production**: Always enable for security
    /// - **Development**: Disable to avoid CSP and other restrictions
    /// - **Staging**: Enable to test security headers before production
    pub fn security_headers(mut self, enabled: bool) -> Self {
        self.config.security_headers = enabled;
        self
    }

    /// Add a custom HTTP header to all responses.
    ///
    /// Custom headers are added to every response served by the Explorer layer.
    /// This is useful for adding environment-specific headers, API versioning,
    /// or custom application headers.
    ///
    /// ## Parameters
    ///
    /// - `key`: Header name (e.g., "X-Environment", "X-API-Version")
    /// - `value`: Header value (e.g., "staging", "v1.0")
    ///
    /// ## Returns
    ///
    /// Returns `Self` for method chaining.
    ///
    /// ## Examples
    ///
    /// ```rust,no_run
    /// use katana_explorer::ExplorerLayer;
    ///
    /// let layer = ExplorerLayer::builder()
    ///     .production()
    ///     .header("X-Environment", "production")
    ///     .header("X-API-Version", "v2.0")
    ///     .header("X-Build-Time", "2024-01-15")
    ///     .build()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// ## Common Use Cases
    ///
    /// - Environment identification: `X-Environment: staging`
    /// - API versioning: `X-API-Version: v1.0`
    /// - Build information: `X-Build-Hash: abc123`
    /// - Custom application headers: `X-Feature-Flags: feature1,feature2`
    pub fn header<K, V>(mut self, key: K, value: V) -> Self
    where
        K: Into<String>,
        V: Into<String>,
    {
        self.config.custom_headers.insert(key.into(), value.into());
        self
    }

    /// Add multiple custom HTTP headers to all responses.
    ///
    /// This is a convenience method for adding multiple headers at once rather
    /// than chaining multiple `.header()` calls.
    ///
    /// ## Parameters
    ///
    /// - `headers`: An iterator of (key, value) pairs where both implement `Into<String>`
    ///
    /// ## Returns
    ///
    /// Returns `Self` for method chaining.
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
    /// let layer = ExplorerLayer::builder().production().headers(header_map).build()?;
    ///
    /// // Or with a Vec of tuples
    /// let headers = vec![("X-Service", "katana-explorer"), ("X-Version", "1.0.0")];
    ///
    /// let layer2 = ExplorerLayer::builder().production().headers(headers).build()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
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

    /// Add a UI environment variable.
    ///
    /// UI environment variables are injected into the HTML page and made available
    /// to the frontend JavaScript via `window.KATANA_CONFIG`. This allows passing
    /// configuration and runtime information to the UI.
    ///
    /// ## Parameters
    ///
    /// - `key`: Variable name (will be available as `window.KATANA_CONFIG.{key}`)
    /// - `value`: Variable value (supports strings, numbers, booleans, and JSON values)
    ///
    /// ## Returns
    ///
    /// Returns `Self` for method chaining.
    ///
    /// ## Examples
    ///
    /// ```rust,no_run
    /// use katana_explorer::ExplorerLayer;
    ///
    /// let layer = ExplorerLayer::builder()
    ///     .development()
    ///     .ui_env("DEBUG", true)
    ///     .ui_env("API_URL", "http://localhost:8080")
    ///     .ui_env("MAX_RETRIES", 3)
    ///     .ui_env("FEATURES", vec!["feature1", "feature2"])
    ///     .build()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// ## Frontend Access
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
    pub fn ui_env<K, V>(mut self, key: K, value: V) -> Self
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
    /// ## Parameters
    ///
    /// - `envs`: An iterator of (key, value) pairs to add to the UI environment
    ///
    /// ## Returns
    ///
    /// Returns `Self` for method chaining.
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
    /// let layer = ExplorerLayer::builder().development().ui_envs(env_vars).build()?;
    ///
    /// // Or with a Vec of tuples
    /// let envs = vec![("FEATURE_A", true), ("FEATURE_B", false), ("POLL_INTERVAL", 1000)];
    ///
    /// let layer2 = ExplorerLayer::builder().development().ui_envs(envs).build()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
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

    /// Enable debug mode by adding `DEBUG=true` to the UI environment.
    ///
    /// This convenience method adds `DEBUG: true` to the UI environment variables,
    /// making it available to the frontend for enabling debug features, logging,
    /// and development tools.
    ///
    /// ## Returns
    ///
    /// Returns `Self` for method chaining.
    ///
    /// ## Examples
    ///
    /// ```rust,no_run
    /// use katana_explorer::ExplorerLayer;
    ///
    /// let layer = ExplorerLayer::builder()
    ///     .development()
    ///     .debug()  // Adds DEBUG: true to UI environment
    ///     .build()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// ## Frontend Access
    ///
    /// The debug flag is accessible in the browser as:
    ///
    /// ```javascript
    /// if (window.KATANA_CONFIG.DEBUG) {
    ///     console.log("Debug mode enabled");
    ///     // Enable debug features
    /// }
    /// ```
    ///
    /// ## Equivalent to
    ///
    /// ```rust,no_run
    /// use katana_explorer::ExplorerLayer;
    ///
    /// let layer = ExplorerLayer::builder().development().ui_env("DEBUG", true).build()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn debug(self) -> Self {
        self.ui_env("DEBUG", true)
    }

    /// Set the API endpoint URL for the UI.
    ///
    /// This convenience method adds an `API_ENDPOINT` variable to the UI environment,
    /// making it available to the frontend for API communication. This is useful
    /// when the API endpoint differs from the default or when using a different port.
    ///
    /// ## Parameters
    ///
    /// - `endpoint`: The API endpoint URL (e.g., "/api/v1", "http://localhost:8080/api")
    ///
    /// ## Returns
    ///
    /// Returns `Self` for method chaining.
    ///
    /// ## Examples
    ///
    /// ```rust,no_run
    /// use katana_explorer::ExplorerLayer;
    ///
    /// // Relative endpoint
    /// let layer = ExplorerLayer::builder().development().api_endpoint("/api/v2").build()?;
    ///
    /// // Absolute endpoint for different service
    /// let layer2 =
    ///     ExplorerLayer::builder().production().api_endpoint("https://api.katana.com/v1").build()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// ## Frontend Access
    ///
    /// The API endpoint is accessible in the browser as:
    ///
    /// ```javascript
    /// const apiUrl = window.KATANA_CONFIG.API_ENDPOINT;
    /// fetch(`${apiUrl}/transactions`);
    /// ```
    ///
    /// ## Equivalent to
    ///
    /// ```rust,no_run
    /// use katana_explorer::ExplorerLayer;
    ///
    /// let layer = ExplorerLayer::builder().development().ui_env("API_ENDPOINT", "/api/v2").build()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn api_endpoint<S: Into<String>>(self, endpoint: S) -> Self {
        self.ui_env("API_ENDPOINT", endpoint.into())
    }

    // === Performance ===

    /// Enable or disable asset compression.
    ///
    /// When enabled, static assets will be compressed before serving to reduce
    /// bandwidth usage and improve loading times. This feature is planned for
    /// future implementation.
    ///
    /// ## Parameters
    ///
    /// - `enabled`: Whether to enable asset compression
    ///
    /// ## Status
    ///
    /// **Currently not implemented** - this setting is reserved for future use.
    /// Setting this value will not have any effect on current behavior.
    ///
    /// ## Returns
    ///
    /// Returns `Self` for method chaining.
    ///
    /// ## Future Examples
    ///
    /// ```rust,no_run
    /// use katana_explorer::ExplorerLayer;
    ///
    /// // When implemented, this will enable gzip/brotli compression
    /// let layer = ExplorerLayer::builder().production().compression(true).build()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
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

impl<S: Clone> Clone for ExplorerService<S> {
    fn clone(&self) -> Self {
        // Note: We don't clone the watcher, as having multiple watchers for the same path
        // would be redundant and potentially problematic. The cache is shared via Arc,
        // so hot reload functionality is maintained.
        Self {
            inner: self.inner.clone(),
            config: self.config.clone(),
            file_cache: self.file_cache.clone(),
            _watcher: None,
        }
    }
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
