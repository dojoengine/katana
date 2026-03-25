//! Versioned RPC module support.
//!
//! Allows mounting different [`RpcModule`] sets at different URL path prefixes,
//! enabling versioned APIs (e.g., `/rpc/v0_9`, `/rpc/v0_10`).

use jsonrpsee::RpcModule;

/// A collection of versioned RPC modules with a default module.
///
/// The `default` module is used for requests that don't match any version prefix.
/// Each versioned module is associated with a path prefix (e.g., `/rpc/v0_9`).
///
/// A single module with no versioned paths is the trivial case — all requests
/// go to the default module.
#[derive(Debug, Clone)]
pub struct VersionedRpcModules {
    /// Default module for requests to `/` or unmatched paths.
    pub default: RpcModule<()>,
    /// Versioned modules, keyed by path prefix (e.g., "/rpc/v0_9").
    pub versioned: Vec<(String, RpcModule<()>)>,
}

impl VersionedRpcModules {
    pub fn new(default: RpcModule<()>) -> Self {
        Self { default, versioned: Vec::new() }
    }

    /// Add a versioned module at the given path prefix.
    pub fn add_version(mut self, path: impl Into<String>, module: RpcModule<()>) -> Self {
        self.versioned.push((path.into(), module));
        self
    }
}

impl From<RpcModule<()>> for VersionedRpcModules {
    fn from(module: RpcModule<()>) -> Self {
        Self::new(module)
    }
}
