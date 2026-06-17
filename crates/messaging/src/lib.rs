#![cfg_attr(not(test), warn(unused_crate_dependencies))]

//! Messaging module.
//!
//! The messaging component is decomposed into two orthogonal concerns:
//!
//! - **Collector** ([`collector::MessageCollector`]): knows *how* to fetch messages from a specific
//!   settlement chain (Ethereum logs, Starknet events, etc).
//! - **Trigger** ([`trigger::MessageTrigger`]): knows *when* to check for new messages (fixed
//!   interval, block subscription, etc).
//!
//! These are composed by [`stream::MessageStream`] into a [`Stream`] that yields
//! [`MessagingOutcome`] items. The stream is consumed by [`server::MessagingServer`]
//! which adds transactions to the pool and persists checkpoints.

pub mod service;
pub mod stream;

use futures::Stream;
use katana_primitives::ContractAddress;
use serde::{Deserialize, Serialize};
pub use service::{MessagingService, MessagingServiceHandle};
use url::Url;

use crate::stream::collector::OrderedMessage;

pub(crate) const LOG_TARGET: &str = "messaging";

/// The outcome yielded by a messenger stream on each successful gather.
#[derive(Debug)]
pub struct MessagingOutcome {
    /// The last settlement block inspected. After the server finishes processing this outcome,
    /// the stream's internal cursor advances past `settlement_block`.
    pub settlement_block: u64,
    /// Positioned messages gathered from the settlement chain, in ascending order.
    /// Each carries its `(block, tx_index)` so the server can write a fine-grained
    /// checkpoint after each successful pool insert.
    pub messages: Vec<OrderedMessage>,
}

/// A messenger is a stream that yields batches of L1Handler transactions
/// gathered from a settlement chain.
///
/// This trait is object-safe, allowing `Box<dyn Messenger>` usage.
pub trait Messenger: Stream<Item = MessagingOutcome> + Send + Unpin {}
impl<T> Messenger for T where T: Stream<Item = MessagingOutcome> + Send + Unpin {}

/// The config used to initialize the messaging service.
#[derive(Debug, Deserialize, Clone, Serialize, PartialEq, Eq)]
pub struct MessagingConfig {
    /// The settlement chain configuration.
    #[serde(flatten)]
    pub settlement: SettlementChainConfig,
    /// The interval, in seconds, at which the messaging service will poll for
    /// new blocks on the settlement chain.
    pub interval: u64,
    /// The block on settlement chain from where Katana will start fetching messages.
    /// Used only on first start. On restart, the persisted checkpoint takes precedence
    /// unless [`force_refetch`](Self::force_refetch) is set.
    pub from_block: u64,
    /// Force fetching from [`from_block`](Self::from_block), ignoring any persisted
    /// checkpoint. Use to re-fetch all messages from the starting block on restart.
    #[serde(default)]
    pub force_refetch: bool,
    /// Number of confirmations to wait before considering a settlement chain block safe
    /// to gather messages from. The messenger only inspects blocks at or below
    /// `latest_block - confirmation_depth`.
    ///
    /// This protects against reorgs: a message gathered from a block that later gets
    /// reorg'd off the canonical chain would otherwise leave the L2 with a tx that has
    /// no L1 origin. Recommended values: ~6-12 for Ethereum L1, 1 for Starknet (single
    /// block finality). Defaults to 0 (no protection — appropriate for dev/test only).
    #[serde(default)]
    pub confirmation_depth: u64,
}

/// Settlement chain configuration with typed variants.
#[derive(Debug, Deserialize, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "chain")]
pub enum SettlementChainConfig {
    #[serde(rename = "ethereum")]
    Ethereum {
        /// The RPC URL of the Ethereum network that the messaging service will listen to.
        rpc_url: Url,
        /// The messaging contract address deployed on the Ethereum network.
        contract_address: katana_primitives::eth::Address,
    },

    #[serde(rename = "starknet")]
    Starknet {
        /// The RPC URL of the Starknet network that the messaging service will listen to.
        rpc_url: Url,
        /// The messaging contract address deployed on the Starknet network.
        contract_address: ContractAddress,
    },
}
