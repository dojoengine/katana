//! Path-based JSON-RPC module router.

use jsonrpsee::RpcModule;

/// Maps URL path prefixes to JSON-RPC modules.
///
/// Routes are matched by prefix in registration order (first match wins).
/// Use [`nest`](Self::nest) to group routes under a common prefix.
///
/// ```rust,ignore
/// use jsonrpsee::RpcModule;
///
/// let router = RpcRouter::new()
///     .route("/", v09_module.clone())
///     .nest("/rpc", RpcRouter::new()
///         .route("/v0_9", v09_module)
///         .route("/v0_10", v010_module)
///     );
/// // Equivalent to: "/", "/rpc/v0_9", "/rpc/v0_10"
/// ```
#[derive(Debug, Default, Clone)]
pub struct RpcRouter {
    pub(crate) routes: Vec<(String, RpcModule<()>)>,
}

impl RpcRouter {
    pub fn new() -> Self {
        Self { routes: Vec::new() }
    }

    /// Register a module at the given path prefix.
    pub fn route(mut self, path: impl Into<String>, module: RpcModule<()>) -> Self {
        self.routes.push((path.into(), module));
        self
    }

    /// Nest another router under a path prefix.
    ///
    /// All routes in `router` are prepended with `prefix`:
    ///
    /// ```rust,ignore
    /// // These two are equivalent:
    /// RpcRouter::new().nest("/rpc", RpcRouter::new().route("/v0_9", m));
    /// RpcRouter::new().route("/rpc/v0_9", m);
    /// ```
    pub fn nest(mut self, prefix: impl Into<String>, router: RpcRouter) -> Self {
        let prefix = prefix.into();
        for (path, module) in router.routes {
            self.routes.push((format!("{prefix}{path}"), module));
        }
        self
    }

    /// Merge another router's routes into this one (no prefix prepended).
    pub fn merge(mut self, other: RpcRouter) -> Self {
        self.routes.extend(other.routes);
        self
    }
}

/// Allow constructing from a single module (mounts at `/`).
impl From<RpcModule<()>> for RpcRouter {
    fn from(module: RpcModule<()>) -> Self {
        Self::new().route("/", module)
    }
}
