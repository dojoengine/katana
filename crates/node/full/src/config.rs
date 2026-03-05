pub use katana_node_config::{db, metrics, rpc};

pub mod trie {
    /// Configuration for state trie computation.
    #[derive(Debug, Clone)]
    pub struct TrieConfig {
        /// Whether to compute and verify state roots during synchronization.
        pub compute: bool,
    }

    impl Default for TrieConfig {
        fn default() -> Self {
            Self { compute: true }
        }
    }
}
