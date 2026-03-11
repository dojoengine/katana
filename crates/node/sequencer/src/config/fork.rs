use katana_primitives::block::BlockHashOrNumber;
use url::Url;

/// Node forking configurations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForkingConfig {
    /// The JSON-RPC URL of the network to fork from.
    pub url: Url,
    /// The block number to fork from. If `None`, the latest block will be used.
    pub block: Option<BlockHashOrNumber>,
    /// Whether to bootstrap local dev genesis allocations on top of the forked state.
    ///
    /// This is typically enabled in developer mode so predeployed dev accounts are available.
    /// Disable it for strict lazy-fetch forking where local state roots must match the remote.
    pub init_dev_genesis: bool,
}
