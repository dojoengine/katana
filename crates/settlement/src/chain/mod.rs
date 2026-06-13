//! Settlement chains.
//!
//! A [`SettlementChain`] is the chain (and core contract) that state updates
//! are submitted to. Its associated [`StateUpdate`](SettlementChain::StateUpdate)
//! type is the payload its core contract accepts, which is what statically
//! ties a [`ProvingBackend`](crate::backend::ProvingBackend) to a compatible
//! chain: a backend can only be paired with a chain whose payload type it
//! produces.
//!
//! [`starknet::StarknetSettlementChain`] (the Piltover core contract on a
//! Starknet chain) is the only implementation today; settling to Ethereum
//! would be a second implementation with its own core contract binding, RPC
//! client (alloy), and payload type.

pub mod starknet;

use async_trait::async_trait;
use katana_primitives::block::BlockNumber;

/// A chain that Katana can settle to.
///
/// Implementations own their RPC client, the settlement account used to sign
/// `update_state` transactions, and the binding to the chain's core contract.
#[async_trait]
pub trait SettlementChain: Send + Sync {
    /// The state-update payload the chain's core contract accepts.
    type StateUpdate: Send + 'static;

    /// Human-readable chain name, for logs.
    fn name(&self) -> &'static str;

    /// Reads the settled block number from the core contract.
    ///
    /// Returns `None` when nothing has been settled yet.
    async fn settled_block(&self) -> Result<Option<BlockNumber>, anyhow::Error>;

    /// Submits the state update to the core contract and waits for the
    /// transaction to be confirmed. Returns a displayable transaction id.
    async fn update_state(&self, update: Self::StateUpdate) -> Result<String, anyhow::Error>;
}
