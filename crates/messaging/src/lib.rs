#![cfg_attr(not(test), warn(unused_crate_dependencies))]

//! Messaging module.
//!
//! The messaging component is decomposed into two orthogonal concerns:
//!
//! - **Collector** ([`collector::MessageCollector`]): knows *how* to fetch messages from a
//!   specific settlement chain (Ethereum logs, Starknet events, etc).
//! - **Trigger** ([`trigger::MessageTrigger`]): knows *when* to check for new messages
//!   (fixed interval, block subscription, etc).
//!
//! These are composed by [`stream::MessageStream`] into a [`Stream`] that yields
//! [`MessagingOutcome`] items. The stream is consumed by [`server::MessagingServer`]
//! which adds transactions to the pool and persists checkpoints.

pub mod collector;
pub mod ethereum;
pub mod server;
pub mod starknet;
pub mod stream;
pub mod trigger;

use std::pin::Pin;
use std::task::{Context, Poll};

use ::starknet::providers::ProviderError as StarknetProviderError;
use alloy_transport::TransportError;
use futures::Stream;
use katana_primitives::chain::ChainId;
use katana_primitives::transaction::L1HandlerTx;
use serde::{Deserialize, Serialize};

use crate::ethereum::EthereumCollector;
use crate::starknet::StarknetCollector;
use crate::stream::MessageStream;
use crate::trigger::IntervalTrigger;

pub(crate) const LOG_TARGET: &str = "messaging";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to initialize messaging")]
    InitError,
    #[error("Unsupported settlement chain")]
    UnsupportedChain,
    #[error("Failed to gather messages from settlement chain")]
    GatherError,
    #[error(transparent)]
    Provider(ProviderError),
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("Ethereum provider error: {0}")]
    Ethereum(TransportError),
    #[error("Starknet provider error: {0}")]
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
    /// The latest settlement block gathered up to.
    pub settlement_block: u64,
    /// The transaction index within `settlement_block` up to which messages were gathered.
    pub tx_index: u64,
    /// The L1Handler transactions gathered from the settlement chain.
    pub transactions: Vec<L1HandlerTx>,
}

/// A messenger is a stream that yields batches of L1Handler transactions
/// gathered from a settlement chain.
///
/// This trait is object-safe, allowing `Box<dyn Messenger>` usage.
pub trait Messenger: Stream<Item = MessagingOutcome> + Send + Unpin {}
impl<T> Messenger for T where T: Stream<Item = MessagingOutcome> + Send + Unpin {}

/// A no-op messenger that never yields any messages.
/// Used when messaging is disabled.
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
    /// Used only if no checkpoint exists in the database.
    pub from_block: u64,
}

/// Settlement chain configuration with typed variants.
#[derive(Debug, Deserialize, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "chain")]
pub enum SettlementChainConfig {
    #[serde(rename = "ethereum")]
    Ethereum {
        rpc_url: String,
        contract_address: String,
    },
    #[serde(rename = "starknet")]
    Starknet {
        rpc_url: String,
        contract_address: String,
    },
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
                    rpc_url: rpc_url.to_string(),
                    contract_address: core_contract.to_string(),
                },
                from_block: *block,
                interval: 2,
            },
            katana_chain_spec::SettlementLayer::Starknet {
                rpc_url, core_contract, block, ..
            } => Self {
                settlement: SettlementChainConfig::Starknet {
                    rpc_url: rpc_url.to_string(),
                    contract_address: core_contract.to_string(),
                },
                from_block: *block,
                interval: 2,
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
/// `from_block` is the starting block on the settlement chain to gather from. Callers should
/// source this from a persisted checkpoint if one exists, falling back to `config.from_block`.
pub fn build_messenger(
    config: Option<&MessagingConfig>,
    chain_id: ChainId,
    from_block: u64,
) -> anyhow::Result<Box<dyn Messenger>> {
    let Some(config) = config else {
        return Ok(Box::new(NoopMessenger));
    };

    let trigger = IntervalTrigger::new(config.interval);

    let stream: Box<dyn Messenger> = match &config.settlement {
        SettlementChainConfig::Ethereum { rpc_url, contract_address } => {
            let collector = EthereumCollector::new(rpc_url, contract_address)?;
            Box::new(MessageStream::new(collector, trigger, chain_id, from_block))
        }
        SettlementChainConfig::Starknet { rpc_url, contract_address } => {
            let collector = StarknetCollector::new(rpc_url, contract_address)?;
            Box::new(MessageStream::new(collector, trigger, chain_id, from_block))
        }
    };

    Ok(stream)
}
