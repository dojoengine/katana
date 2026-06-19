//! Node settlement config.
//!
//! Settlement is orthogonal to the chain spec, so the node holds it as its own
//! [`SettlementConfig`] (re-exported from `katana-chain-spec`) rather than
//! inside `ChainSpec`. The embedded settlement service derives its narrower,
//! Starknet/TEE-specific config from this via
//! [`katana_settlement::SettlementConfig::from_node_config`].

pub use katana_chain_spec::{
    SettlementConfig, SettlementLayer, SettlementProofKind, SettlementRuntime,
};
