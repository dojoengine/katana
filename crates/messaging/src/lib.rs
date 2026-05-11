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

pub mod server;
pub mod stream;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

use std::pin::Pin;
use std::task::{Context, Poll};

use ::starknet::providers::ProviderError as StarknetProviderError;
use alloy_transport::TransportError;
use futures::Stream;
use katana_primitives::chain::ChainId;
use katana_primitives::ContractAddress;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::stream::collector::ethereum::EthereumCollector;
use crate::stream::collector::starknet::StarknetCollector;
use crate::stream::collector::OrderedMessage;
use crate::stream::trigger::IntervalTrigger;
use crate::stream::MessageStream;

pub(crate) const LOG_TARGET: &str = "messaging";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to initialize messaging")]
    InitError,

    #[error("unsupported settlement chain")]
    UnsupportedChain,

    #[error("failed to gather messages from settlement chain")]
    GatherError,

    /// A settlement chain log/event was found whose shape didn't match the expected
    /// schema. Surfaces the specific reason so operators can diagnose contract
    /// upgrades, RPC bugs, or chain-id mismatches without a stack trace.
    #[error("malformed settlement chain message: {0}")]
    MalformedMessage(String),

    #[error(transparent)]
    Provider(ProviderError),
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("ethereum provider error: {0}")]
    Ethereum(TransportError),

    #[error("starknet provider error: {0}")]
    Starknet(StarknetProviderError),
}

impl From<TransportError> for Error {
    fn from(e: TransportError) -> Self {
        Self::Provider(ProviderError::Ethereum(e))
    }
}

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

/// A no-op messenger that never yields any messages.
/// Used when messaging is disabled.
#[derive(Debug)]
pub struct NoopMessenger;

impl Stream for NoopMessenger {
    type Item = MessagingOutcome;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Pending
    }
}

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
    /// Used only on first start. On restart, the persisted checkpoint takes precedence.
    pub from_block: u64,
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
    Ethereum { rpc_url: Url, contract_address: katana_primitives::eth::Address },
    #[serde(rename = "starknet")]
    Starknet { rpc_url: Url, contract_address: ContractAddress },
}

impl MessagingConfig {
    /// Load the config from a JSON file.
    pub fn load(path: impl AsRef<std::path::Path>) -> Result<Self, std::io::Error> {
        let buf = std::fs::read(path)?;
        serde_json::from_slice(&buf).map_err(|e| e.into())
    }

    /// This is used as the clap `value_parser` implementation.
    pub fn parse(path: &str) -> Result<Self, String> {
        Self::load(path).map_err(|e| e.to_string())
    }

    pub fn from_chain_spec(spec: &katana_chain_spec::rollup::ChainSpec) -> Self {
        match &spec.settlement {
            katana_chain_spec::SettlementLayer::Ethereum {
                rpc_url, core_contract, block, ..
            } => Self {
                settlement: SettlementChainConfig::Ethereum {
                    rpc_url: rpc_url.clone(),
                    contract_address: core_contract.clone(),
                },
                from_block: *block,
                interval: 2,
                confirmation_depth: 0,
            },
            katana_chain_spec::SettlementLayer::Starknet {
                rpc_url, core_contract, block, ..
            } => Self {
                settlement: SettlementChainConfig::Starknet {
                    rpc_url: rpc_url.clone(),
                    contract_address: core_contract.clone(),
                },
                from_block: *block,
                interval: 2,
                confirmation_depth: 0,
            },
            katana_chain_spec::SettlementLayer::Sovereign { .. } => {
                panic!("Sovereign chains are not supported for messaging.")
            }
        }
    }
}

/// Build a ready-to-drain messenger from a messaging config.
///
/// Encapsulates settlement chain selection, collector construction, and trigger composition.
/// If `config` is `None`, returns a [`NoopMessenger`] (messaging disabled). Otherwise builds
/// the appropriate collector for the settlement chain, wraps it in a stream driven by an
/// [`IntervalTrigger`], and returns it boxed for type erasure.
///
/// `from_block` / `from_tx_index` form the resume cursor. On a fresh start they come from
/// `config.from_block` and `0`; on restart from a persisted checkpoint they come from the
/// last processed message's position (with `from_tx_index` incremented to start after it).
pub fn build_messenger(
    config: Option<&MessagingConfig>,
    chain_id: ChainId,
    from_block: u64,
    from_tx_index: u64,
) -> anyhow::Result<Box<dyn Messenger>> {
    let Some(config) = config else {
        return Ok(Box::new(NoopMessenger));
    };

    let trigger = IntervalTrigger::new(config.interval);
    let confirmation_depth = config.confirmation_depth;

    let stream: Box<dyn Messenger> = match &config.settlement {
        SettlementChainConfig::Ethereum { rpc_url, contract_address } => {
            let collector = EthereumCollector::new(rpc_url.clone(), *contract_address)?;
            Box::new(MessageStream::with_cursor(
                collector,
                trigger,
                chain_id,
                from_block,
                from_tx_index,
                confirmation_depth,
            ))
        }

        SettlementChainConfig::Starknet { rpc_url, contract_address } => {
            let collector = StarknetCollector::new(rpc_url.clone(), *contract_address)?;
            Box::new(MessageStream::with_cursor(
                collector,
                trigger,
                chain_id,
                from_block,
                from_tx_index,
                confirmation_depth,
            ))
        }
    };

    Ok(stream)
}
