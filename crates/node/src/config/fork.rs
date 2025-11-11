use katana_primitives::block::BlockIdOrTag;
use url::Url;

/// Node forking configurations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForkingConfig {
    /// The JSON-RPC URL of the network to fork from.
    pub url: Url,
    /// The block id or tag to fork from. If `None`, the latest block will be used.
    pub block: Option<BlockIdOrTag>,
}
